use uuid::Uuid;
use worker::Env;

/// Telegram Bot API KEY
pub const ENV_API_KEY: &str = "API_KEY";
/// YNAB API KEY
pub const ENV_YNAB_API_KEY: &str = "YNAB_API_KEY";
/// YNAB Budget ID
///
/// `last-used` can be used to specify the last used budget
pub const ENV_YNAB_BUDGET_ID: &str = "YNAB_BUDGET_ID";
/// YNAB Account ID
pub const ENV_YNAB_ACCOUNT_ID: &str = "YNAB_ACCOUNT_ID";
/// Webhook API Key for authentication
pub const ENV_WEBHOOK_API_KEY: &str = "WEBHOOK_API_KEY";

pub struct Config {
    pub tg_api_key: String,
    pub ynab_api_key: String,
    pub ynab_budget_id: String,
    pub ynab_account_id: Uuid,
    pub webhook_api_key: String,
}

pub fn init_config(env: &Env) -> worker::Result<Config> {
    let tg_api_key = env.secret(ENV_API_KEY)?.to_string();
    let ynab_api_key = env.secret(ENV_YNAB_API_KEY)?.to_string();
    let ynab_budget_id = env.secret(ENV_YNAB_BUDGET_ID)?.to_string();
    let ynab_account_id = env
        .secret(ENV_YNAB_ACCOUNT_ID)?
        .to_string()
        .parse::<Uuid>()
        .map_err(|err| worker::Error::RustError(err.to_string()))?;
    let webhook_api_key = env.secret(ENV_WEBHOOK_API_KEY)?.to_string();

    Ok(Config {
        tg_api_key,
        ynab_api_key,
        ynab_budget_id,
        ynab_account_id,
        webhook_api_key,
    })
}
