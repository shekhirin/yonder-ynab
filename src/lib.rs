use std::{fmt::Display, io::Cursor, sync::Arc};

use chrono::NaiveDateTime;
use eyre::{Context, OptionExt};
use futures::TryFutureExt;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use tgbot_worker_rs::{
    frankenstein::{methods::GetFileParams, AsyncTelegramApi},
    App, Bot, BotError, Message,
};
use worker::{event, Env, Request, Response};

use crate::ynab::types::{NewTransaction, PostTransactionsWrapper, TransactionClearedStatus};

mod config;
use config::{init_config, Config};

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

#[derive(Serialize)]
struct DocumentResult {
    imported: usize,
    duplicates: usize,
}

impl Display for DocumentResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Imported new transactions: {}\nSkipped duplicate transactions: {}",
            self.imported, self.duplicates
        )
    }
}

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, ctx: worker::Context) -> worker::Result<Response> {
    let config = init_config(&env)?;

    let ynab_client = ynab::Client::new_with_client(
        "https://api.ynab.com/v1",
        reqwest::ClientBuilder::new()
            .default_headers(HeaderMap::from_iter([(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", config.ynab_api_key).parse()?,
            )]))
            .build()
            .map_err(|err| worker::Error::RustError(err.to_string()))?,
    );

    let config = Arc::new(config);
    let ynab_client = Arc::new(ynab_client);

    if req.path() == "/import" {
        // Handle custom webhook
        on_webhook_import(req, config, ynab_client).await
    } else {
        // Handle Telegram bot webhook
        let mut app = App::new();

        let config_clone = config.clone();
        let ynab_client_clone = ynab_client.clone();

        app.on_message(move |bot, msg| {
            on_telegram_message(config_clone.clone(), ynab_client_clone.clone(), bot, msg)
                .map_err(|err| BotError::Custom(err.to_string()))
        });

        app.run(req, env, ctx).await
    }
}

/// Handle Telegram message
async fn on_telegram_message(
    config: Arc<Config>,
    ynab_client: Arc<ynab::Client>,
    bot: Bot,
    msg: Message,
) -> eyre::Result<()> {
    let Some(document) = msg.inner().document.clone() else {
        bot.send_message(msg.chat_id(), "Send Yonder CSV export as a document")
            .await?;
        return Ok(());
    };

    match on_telegram_document(config, ynab_client, bot.clone(), document.file_id).await {
        Ok(result) => bot.send_message(msg.chat_id(), &result.to_string()).await?,
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

/// Handle Telegram document
async fn on_telegram_document(
    config: Arc<Config>,
    ynab_client: Arc<ynab::Client>,
    bot: Bot,
    file_id: String,
) -> eyre::Result<DocumentResult> {
    let tg_api_key = config
        .tg_api_key
        .as_deref()
        .ok_or_eyre("Telegram API key is not set")?;

    // Download file from Telegram
    let file = bot.inner().get_file(&GetFileParams { file_id }).await?;
    let file_path = file.result.file_path.ok_or_eyre("no file path found")?;
    let file_response = bot
        .inner()
        .client
        .get(format!(
            "https://api.telegram.org/file/bot{tg_api_key}/{file_path}",
        ))
        .send()
        .await?;

    let csv_bytes = file_response.bytes().await?;
    import_yonder_csv_to_ynab(csv_bytes, &config, &ynab_client).await
}

/// Handle CSV import via HTTP webhook
async fn on_webhook_import(
    mut req: Request,
    config: Arc<Config>,
    ynab_client: Arc<ynab::Client>,
) -> worker::Result<Response> {
    let api_key = req
        .url()?
        .query_pairs()
        .find_map(|(k, v)| (k == "api_key").then(|| v.into_owned()));

    let Some(webhook_api_key) = config.webhook_api_key.as_deref() else {
        return Response::error("Webhook API key is not set", 401);
    };

    if api_key.as_deref() != Some(webhook_api_key) {
        return Response::error("Invalid API key", 401);
    }

    let csv_bytes = req.bytes().await?;
    match import_yonder_csv_to_ynab(csv_bytes, &config, &ynab_client).await {
        Ok(result) => Response::from_json(&serde_json::json!({"message": result.to_string()})),
        Err(err) => Response::error(err.to_string(), 500),
    }
}

/// Parse Yonder transacitons in CSV format and import to YNAB
async fn import_yonder_csv_to_ynab(
    yonder_csv: impl AsRef<[u8]>,
    config: &Config,
    ynab_client: &ynab::Client,
) -> eyre::Result<DocumentResult> {
    // Parse CSV with Yonder transactions
    let yonder_transactions: Vec<YonderTransaction> =
        csv::Reader::from_reader(Cursor::new(yonder_csv))
            .into_deserialize()
            .collect::<Result<_, _>>()
            .wrap_err("failed to deserialize as Yonder transactions CSV")?;

    // Map Yonder transactions to YNAB format
    let ynab_transactions: Vec<_> = yonder_transactions
        .into_iter()
        .map(NewTransaction::from)
        .map(|mut transaction| {
            transaction.account_id = Some(config.ynab_account_id);
            transaction
        })
        .collect();

    // Import transactions to YNAB
    let ynab_response = ynab_client
        .create_transaction(
            &config.ynab_budget_id,
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
