use super::BotError;
use std::path::PathBuf;
use url::Url;

/// Private credentials for bot-token MTProto login. Never expose them in public config.
#[derive(Clone)]
pub struct BotCredentials {
    pub(super) token: String,
    pub(super) api_id: i32,
    pub(super) api_hash: String,
    pub(super) bot_id: u64,
}
impl BotCredentials {
    pub fn new(token: String, api_id: i32, api_hash: String) -> anyhow::Result<Self> {
        let (id, secret) = token
            .split_once(':')
            .ok_or(BotError::Configuration("Invalid bot token"))?;
        let bot_id = id
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0 && *id <= i64::MAX as u64)
            .ok_or(BotError::Configuration("Invalid bot token"))?;
        anyhow::ensure!(
            secret.len() >= 16
                && secret
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "Invalid bot token"
        );
        anyhow::ensure!(api_id > 0, "TELEGRAM_API_ID must be positive");
        anyhow::ensure!(
            api_hash.len() == 32 && api_hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "TELEGRAM_API_HASH must contain 32 hexadecimal characters"
        );
        Ok(Self {
            token,
            api_id,
            api_hash,
            bot_id,
        })
    }
    pub(super) fn bot_id(&self) -> u64 {
        self.bot_id
    }
}

pub struct BotConfig {
    pub credentials: BotCredentials,
    pub public_url: Option<String>,
    pub data_dir: PathBuf,
}

// Launch validation is independent of Mini App authentication.
pub(super) fn app_url(raw: &str) -> anyhow::Result<Url> {
    let url = Url::parse(raw).map_err(|_| anyhow::anyhow!("Invalid public app URL"))?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "Public app URL must be an HTTPS origin without credentials, query or fragment"
    );
    Ok(url)
}
