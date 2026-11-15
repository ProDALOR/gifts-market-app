//! A small Bot API client and one polling worker. It never reads Telegram account exports.
use fs2::FileExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Transport,
    Checkpoint,
    InvalidResponse,
    Api(u16),
    RetryAfter(u64),
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Checkpoint => f.write_str("Bot checkpoint storage unavailable"),
            Self::Transport => f.write_str("Telegram connection failed"),
            Self::InvalidResponse => f.write_str("Invalid Bot API response"),
            Self::Api(code) => write!(f, "Telegram rejected request ({code})"),
            Self::RetryAfter(seconds) => write!(f, "Telegram rate limit; retry in {seconds}s"),
        }
    }
}
impl std::error::Error for Failure {}
impl Failure {
    pub fn delay(&self, fallback: u64) -> Duration {
        Duration::from_secs(match self {
            Self::RetryAfter(seconds) => (*seconds).clamp(1, 3600),
            _ => fallback,
        })
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BotUser {
    pub id: u64,
    pub username: String,
    pub first_name: String,
    #[serde(default)]
    pub has_main_web_app: bool,
}
#[derive(Deserialize)]
pub struct Webhook {
    pub url: String,
}
#[derive(Deserialize)]
struct Parameters {
    retry_after: Option<u64>,
}
#[derive(Deserialize)]
struct Envelope<T> {
    ok: bool,
    result: Option<T>,
    error_code: Option<u16>,
    parameters: Option<Parameters>,
}
#[derive(Debug, Deserialize)]
pub struct Update {
    pub update_id: u64,
    pub message: Option<Message>,
}
#[derive(Debug, Deserialize)]
pub struct Message {
    pub chat: Chat,
    pub text: Option<String>,
}
#[derive(Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
}
#[derive(Clone)]
pub struct Bot {
    client: reqwest::Client,
    base: Url,
}
impl Bot {
    pub fn new(token: &str) -> anyhow::Result<Self> {
        let (id, secret) = token
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("Invalid bot token"))?;
        anyhow::ensure!(
            id.parse::<u64>().is_ok_and(|id| id > 0)
                && secret.len() >= 16
                && secret
                    .bytes()
                    .all(|v| v.is_ascii_alphanumeric() || b"_-".contains(&v)),
            "Invalid bot token"
        );
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            base: Url::parse(&format!("https://api.telegram.org/bot{token}/"))
                .map_err(|_| anyhow::anyhow!("Invalid bot token"))?,
        })
    }
    async fn call<T: DeserializeOwned>(&self, method: &str, body: Value) -> Result<T, Failure> {
        let url = self
            .base
            .join(method)
            .map_err(|_| Failure::InvalidResponse)?;
        // Do not propagate reqwest errors: their URLs contain the bot credential.
        let mut response = self
            .client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|_| Failure::Transport)?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Transport)? {
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                return Err(Failure::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        let envelope: Envelope<T> =
            serde_json::from_slice(&bytes).map_err(|_| Failure::InvalidResponse)?;
        if !envelope.ok {
            if let Some(seconds) = envelope.parameters.and_then(|p| p.retry_after) {
                return Err(Failure::RetryAfter(seconds));
            }
            return Err(Failure::Api(envelope.error_code.unwrap_or(500)));
        }
        envelope.result.ok_or(Failure::InvalidResponse)
    }
    pub async fn me(&self) -> Result<BotUser, Failure> {
        self.call("getMe", json!({})).await
    }
    pub async fn webhook(&self) -> Result<Webhook, Failure> {
        self.call("getWebhookInfo", json!({})).await
    }
    pub async fn configure(&self, url: &Url) -> Result<(), Failure> {
        let _:bool=self.call("setMyCommands",json!({"commands":[{"command":"start","description":"Open gift markets"},{"command":"market","description":"Browse markets"},{"command":"help","description":"How to use the app"}]})).await?;
        let _:bool=self.call("setChatMenuButton",json!({"menu_button":{"type":"web_app","text":"Open market","web_app":{"url":url.as_str()}}})).await?;
        Ok(())
    }
    pub async fn updates(&self, offset: u64) -> Result<Vec<Update>, Failure> {
        self.call(
            "getUpdates",
            json!({"offset":offset,"limit":50,"timeout":30,"allowed_updates":["message"]}),
        )
        .await
    }
    pub async fn handle(&self, update: &Update, username: &str, url: &Url) -> Result<(), Failure> {
        let Some(message) = &update.message else {
            return Ok(());
        };
        if message.chat.kind != "private" {
            return Ok(());
        }
        let Some(command) = message
            .text
            .as_deref()
            .and_then(|v| v.split_whitespace().next())
        else {
            return Ok(());
        };
        let (command, target) = command
            .split_once('@')
            .map_or((command, None), |(command, target)| (command, Some(target)));
        if target.is_some_and(|target| !target.eq_ignore_ascii_case(username)) {
            return Ok(());
        }
        let text = match command {
            "/start" | "/market" => {
                "Browse live gift markets, compare venues and filter offers in one place."
            }
            "/help" => {
                "Open the market to choose a collection and currency, compare offers and inspect order details. Use /market to open it again."
            }
            _ => return Ok(()),
        };
        let _:Value=self.call("sendMessage",json!({"chat_id":message.chat.id,"text":text,"reply_markup":{"inline_keyboard":[[{"text":"Open market","web_app":{"url":url.as_str()}}]]}})).await?;
        Ok(())
    }
}
pub fn app_url(raw: &str) -> anyhow::Result<Url> {
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
pub struct Checkpoint {
    path: PathBuf,
    pub offset: u64,
    _lock: File,
}
impl Checkpoint {
    pub fn open(directory: &Path, bot_id: u64) -> anyhow::Result<Self> {
        std::fs::create_dir_all(directory)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(format!("{bot_id}.lock")))?;
        lock.try_lock_exclusive()
            .map_err(|_| anyhow::anyhow!("Another bot worker owns this data directory"))?;
        let path = directory.join(format!("{bot_id}.offset"));
        let offset = if path.exists() {
            u64::from_be_bytes(
                std::fs::read(&path)?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Invalid bot update checkpoint"))?,
            )
        } else {
            0
        };
        Ok(Self {
            path,
            offset,
            _lock: lock,
        })
    }
    pub fn save(&mut self, offset: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            offset >= self.offset,
            "Bot update checkpoint cannot move backwards"
        );
        let temporary = self.path.with_extension("tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(&offset.to_be_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, &self.path)?;
        File::open(
            self.path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Invalid checkpoint path"))?,
        )?
        .sync_all()?;
        self.offset = offset;
        Ok(())
    }
}
pub async fn run(
    bot: Bot,
    user: BotUser,
    url: Url,
    mut checkpoint: Checkpoint,
    stop: CancellationToken,
) -> anyhow::Result<()> {
    let mut backoff = 1;
    loop {
        let work = async {
            let updates = bot.updates(checkpoint.offset).await?;
            for update in updates {
                if update.update_id < checkpoint.offset {
                    continue;
                }
                match bot.handle(&update, &user.username, &url).await {
                    Ok(()) | Err(Failure::Api(403)) => {}
                    Err(error) => return Err(error),
                }
                let offset = update
                    .update_id
                    .checked_add(1)
                    .ok_or(Failure::InvalidResponse)?;
                checkpoint.save(offset).map_err(|_| Failure::Checkpoint)?;
            }
            Ok::<(), Failure>(())
        };
        let error = tokio::select! {_=stop.cancelled()=>break,result=work=>match result{Ok(())=>{backoff=1;continue;},Err(error)=>error}};
        if matches!(
            error,
            Failure::Api(401) | Failure::Api(409) | Failure::Checkpoint
        ) {
            return Err(error.into());
        }
        eprintln!("{error}");
        tokio::select! {_=stop.cancelled()=>break,_=tokio::time::sleep(error.delay(backoff))=>{}}
        backoff = (backoff * 2).min(30);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use std::sync::{Arc, Mutex};
    #[tokio::test]
    async fn commands_configure_launch_buttons_without_exposing_credentials() {
        let received = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let state = received.clone();
        let app=Router::new().route("/{method}",post(move|axum::extract::Path(method):axum::extract::Path<String>,Json(body):Json<Value>|{let state=state.clone();async move{state.lock().unwrap().push((method.clone(),body));Json(json!({"ok":true,"result":if method=="sendMessage"{json!({"message_id":1})}else{json!(true)}}))}}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut bot = Bot::new("42:private_test_secret").unwrap();
        bot.base = base;
        let url = app_url("https://market.example").unwrap();
        bot.configure(&url).await.unwrap();
        for text in ["/start", "/market@igifts_app", "/help"] {
            bot.handle(
                &Update {
                    update_id: 1,
                    message: Some(Message {
                        chat: Chat {
                            id: 7,
                            kind: "private".into(),
                        },
                        text: Some(text.into()),
                    }),
                },
                "igifts_app",
                &url,
            )
            .await
            .unwrap();
        }
        for (text, kind) in [
            ("/start@another_bot", "private"),
            ("hello", "private"),
            ("/start", "group"),
        ] {
            bot.handle(
                &Update {
                    update_id: 2,
                    message: Some(Message {
                        chat: Chat {
                            id: 7,
                            kind: kind.into(),
                        },
                        text: Some(text.into()),
                    }),
                },
                "igifts_app",
                &url,
            )
            .await
            .unwrap();
        }
        let rows = received.lock().unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(
            rows[1].1["menu_button"]["web_app"]["url"],
            "https://market.example/"
        );
        assert_eq!(
            rows[2].1["reply_markup"]["inline_keyboard"][0][0]["web_app"]["url"],
            "https://market.example/"
        );
        assert!(
            !serde_json::to_string(&*rows)
                .unwrap()
                .contains("private_test_secret")
        );
        server.abort();
    }
    #[test]
    fn polling_checkpoints_are_durable_and_single_writer() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = Checkpoint::open(dir.path(), 42).unwrap();
        assert!(Checkpoint::open(dir.path(), 42).is_err());
        first.save(9).unwrap();
        drop(first);
        let mut restored = Checkpoint::open(dir.path(), 42).unwrap();
        assert_eq!(restored.offset, 9);
        assert!(restored.save(8).is_err());
    }
    #[test]
    fn invalid_urls_tokens_and_errors_do_not_leak_secrets() {
        for value in [
            "http://market.example",
            "https://user:secret@market.example",
            "https://market.example/?token=secret",
            "https://market.example/#secret",
        ] {
            assert!(app_url(value).is_err());
        }
        assert!(Bot::new("secret").is_err());
        assert!(!Failure::Transport.to_string().contains("secret"));
    }
}
