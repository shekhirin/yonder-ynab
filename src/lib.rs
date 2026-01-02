use std::{io::Cursor, sync::Arc};

use chrono::NaiveDateTime;
use eyre::OptionExt;
use futures::TryFutureExt;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use tgbot_worker_rs::{
    frankenstein::{methods::GetFileParams, AsyncTelegramApi},
    App, Bot, BotError, Message,
};
use uuid::Uuid;
use worker::{event, Context, Env, Request, Response};

use crate::ynab::types::{NewTransaction, PostTransactionsWrapper, TransactionClearedStatus};

mod ynab {
    progenitor::generate_api!(spec = "ynab_openapi.yml",);
}

#[derive(Debug, PartialEq, Deserialize)]
struct YonderTransaction {
    #[serde(rename = "Date/Time of transaction")]
    date_time: NaiveDateTime,
    #[serde(rename = "Description")]
    description: String,
    #[serde(rename = "Amount (GBP)")]
    amount_gbp: f64,
    #[serde(rename = "Amount (in Charged Currency)")]
    amount_charged: f64,
    #[serde(rename = "Currency")]
    currency: String,
    #[serde(rename = "Category")]
    category: String,
    #[serde(rename = "Debit or Credit")]
    kind: YonderTransactionKind,
    #[serde(rename = "Country")]
    country: String,
}

impl From<YonderTransaction> for NewTransaction {
    fn from(value: YonderTransaction) -> Self {
        Self {
            account_id: None,
            amount: Some(
                (match value.kind {
                    YonderTransactionKind::Debit => -value.amount_gbp,
                    YonderTransactionKind::Credit => value.amount_gbp,
                } * 1000.0) as i64,
            ),
            approved: None,
            category_id: None,
            cleared: Some(TransactionClearedStatus::Cleared),
            date: Some(value.date_time.and_utc().date_naive()),
            flag_color: None,
            import_id: Some(
                format!(
                    "TG:{}:{}",
                    value.amount_gbp,
                    value.date_time.and_utc().timestamp_millis()
                )
                .parse()
                .unwrap(),
            ),
            memo: None,
            payee_id: None,
            payee_name: Some(value.description.parse().unwrap()),
            subtransactions: vec![],
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
enum YonderTransactionKind {
    Debit,
    Credit,
}

struct DocumentResult {
    imported: usize,
    duplicates: usize,
}

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, ctx: Context) -> worker::Result<Response> {
    let mut app = App::new();

    let tg_api_key = Arc::new(env.secret("API_KEY")?.to_string());
    let ynab_api_key = env.secret("YNAB_API_KEY")?.to_string();
    let ynab_budget_id = env
        .secret("YNAB_BUDGET_ID")
        .map_or("last-used".to_string(), |secret| secret.to_string());

    let ynab_client = ynab::Client::new_with_client(
        "https://api.ynab.com/v1",
        reqwest::ClientBuilder::new()
            .default_headers(HeaderMap::from_iter([(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {ynab_api_key}").parse()?,
            )]))
            .build()
            .map_err(|err| worker::Error::RustError(err.to_string()))?,
    );

    let accounts = ynab_client
        .get_accounts(&ynab_budget_id, None)
        .await
        .map_err(|err| worker::Error::RustError(err.to_string()))?;
    let ynab_account_id = accounts
        .into_inner()
        .data
        .accounts
        .into_iter()
        .find(|account| account.name.eq_ignore_ascii_case("yonder"))
        .map(|account| Ok(account.id))
        .or_else(|| {
            Some(
                env.secret("YNAB_ACCOUNT_ID")
                    .ok()?
                    .to_string()
                    .parse::<Uuid>()
                    .map_err(|err| worker::Error::RustError(err.to_string())),
            )
        })
        .transpose()?
        .ok_or_else(|| {
            worker::Error::RustError(
                "No YNAB account ID is set. Either rename one of the accounts to \"Yonder\" or set the YNAB_ACCOUNT_ID secret."
                    .to_string(),
            )
        })?;

    app.on_message(move |bot, msg| {
        on_message(
            tg_api_key.clone(),
            ynab_client.clone(),
            ynab_account_id,
            bot,
            msg,
        )
        .map_err(|err| BotError::Custom(err.to_string()))
    });

    app.run(req, env, ctx).await
}

async fn on_message(
    tg_api_key: Arc<String>,
    ynab_client: ynab::Client,
    ynab_account_id: Uuid,
    bot: Bot,
    msg: Message,
) -> eyre::Result<()> {
    let Some(document) = msg.inner().document.clone() else {
        bot.send_message(msg.chat_id(), "Send Yonder CSV export as a document")
            .await?;
        return Ok(());
    };

    match on_document(
        tg_api_key,
        ynab_client,
        ynab_account_id,
        bot.clone(),
        document.file_id,
    )
    .await
    {
        Ok(DocumentResult {
            imported,
            duplicates,
        }) => {
            bot.send_message(
                msg.chat_id(),
                &format!("Imported transactions: {imported}\nSkipped duplicates: {duplicates}"),
            )
            .await?
        }
        Err(err) => {
            bot.send_message(
                msg.chat_id(),
                &format!("Failed to import transactions:\n\n{}", err),
            )
            .await?
        }
    }

    Ok(())
}

async fn on_document(
    tg_api_key: Arc<String>,
    ynab_client: ynab::Client,
    ynab_account_id: Uuid,
    bot: Bot,
    file_id: String,
) -> eyre::Result<DocumentResult> {
    let file = bot.inner().get_file(&GetFileParams { file_id }).await?;
    let file_path = file.result.file_path.ok_or_eyre("no file path found")?;
    let file_response = bot
        .inner()
        .client
        .get(format!(
            "https://api.telegram.org/file/bot{tg_api_key}/{file_path}"
        ))
        .send()
        .await?;

    let yonder_transactions: Vec<YonderTransaction> =
        csv::Reader::from_reader(Cursor::new(file_response.bytes().await?))
            .into_deserialize()
            .collect::<Result<_, _>>()?;

    let ynab_transactions: Vec<_> = yonder_transactions
        .into_iter()
        .map(NewTransaction::from)
        .map(|mut transaction| {
            transaction.account_id = Some(ynab_account_id);
            transaction
        })
        .collect();
    let ynab_response = ynab_client
        .create_transaction(
            "last-used",
            &PostTransactionsWrapper {
                transaction: None,
                transactions: ynab_transactions,
            },
        )
        .await
        .map_err(|err| eyre::Report::msg(err.to_string()))?;

    Ok(DocumentResult {
        imported: ynab_response.data.transaction_ids.len(),
        duplicates: ynab_response.data.duplicate_import_ids.len(),
    })
}

#[cfg(test)]
mod tests {
    use crate::{ynab::types::NewTransaction, YonderTransaction, YonderTransactionKind};

    #[test]
    fn test_parse_yonder() -> eyre::Result<()> {
        let yonder_transactions: Vec<YonderTransaction> =
            csv::Reader::from_reader(std::fs::read("yonder.csv")?.as_slice())
                .into_deserialize()
                .collect::<Result<_, _>>()?;

        assert_eq!(
            yonder_transactions,
            vec![YonderTransaction {
                date_time: "2026-01-01T10:34:50.211697".parse()?,
                description: "TFL - Transport for London".to_string(),
                amount_gbp: 3.00,
                amount_charged: 3.00,
                currency: "GBP".to_string(),
                category: "Transport".to_string(),
                kind: YonderTransactionKind::Debit,
                country: "GBR".to_string()
            }]
        );

        Ok(())
    }

    #[test]
    fn test_ynab_import_id_length() -> eyre::Result<()> {
        let yonder_transactions: Vec<YonderTransaction> =
            csv::Reader::from_reader(std::fs::read("yonder.csv")?.as_slice())
                .into_deserialize()
                .collect::<Result<_, _>>()?;

        for transaction in yonder_transactions {
            let import_id = NewTransaction::from(transaction).import_id;
            assert!(
                import_id.expect("import_id must be set").len() < 36,
                "import_id must be no longer than 36 characters"
            );
        }

        Ok(())
    }
}
