//! Translate application settings into the embeddable API service.
use crate::cli::Cli;
use market_actors::MarketDataHandle;
#[cfg(feature = "miniapp")]
use market_api::miniapp::MiniAppApi;
use market_api::{ApiConfig, ApiServer};

pub async fn server(
    args: &Cli,
    data: MarketDataHandle,
    valuation: market_valuation::ValuationHandle,
    #[cfg(feature = "bot")] _bot: Option<&mut market_telegram::TelegramWorker>,
) -> anyhow::Result<ApiServer> {
    let server = ApiServer::bind(
        ApiConfig {
            bind: args.bind,
            workers: args.http_workers,
            web_dir: (!args.no_web).then(|| args.web_dir.clone()),
            cors_origins: args
                .cors_origin
                .iter()
                .map(|origin| origin.trim())
                .filter(|origin| !origin.is_empty())
                .map(str::to_owned)
                .collect(),
            limits: Default::default(),
        },
        data,
        valuation,
    )?;
    #[cfg(feature = "miniapp")]
    let server = server.with_miniapp(
        miniapp(
            args,
            #[cfg(feature = "bot")]
            _bot,
        )
        .await?,
    );
    Ok(server)
}

#[cfg(feature = "miniapp")]
async fn miniapp(
    args: &Cli,
    #[cfg(feature = "bot")] bot: Option<&mut market_telegram::TelegramWorker>,
) -> anyhow::Result<MiniAppApi> {
    #[cfg(feature = "bot")]
    let discover = bot.is_some();
    #[cfg(not(feature = "bot"))]
    let discover = false;
    let bot_id = args.miniapp_bot_id.filter(|id| *id > 0);
    let username = args
        .miniapp_bot_username
        .clone()
        .filter(|name| !name.is_empty());
    anyhow::ensure!(
        !discover || (bot_id.is_none() && username.is_none()),
        "MARKET_MINIAPP_BOT_ID and MARKET_MINIAPP_BOT_USERNAME are only for deployments without a bot worker; enabled workers discover identity after authentication"
    );
    let api = MiniAppApi::new(None, username.clone(), args.public_url.clone())?;
    if discover || bot_id.is_some() {
        let key = args.session_key.clone().filter(|value| !value.is_empty());
        let key_file = args.data_dir.join("session.key");
        let key = tokio::task::spawn_blocking(move || {
            market_miniapp::auth::session_key(&key_file, key.as_deref())
        })
        .await??;
        #[cfg(feature = "bot")]
        if let Some(bot) = bot {
            let api = api.clone();
            bot.on_authenticated(move |user| {
                let result = market_miniapp::auth::Auth::production(user.id, key)
                    .and_then(|auth| api.set_identity(auth, Some(user.username.clone())));
                if let Err(error) = result {
                    tracing::error!(%error, "could not publish Mini App bot identity");
                }
            });
        }
        if let Some(id) = bot_id {
            api.set_identity(market_miniapp::auth::Auth::production(id, key)?, username)?;
        }
    }
    Ok(api)
}

#[cfg(all(test, feature = "miniapp"))]
mod tests {
    use super::*;
    use clap::Parser;

    #[tokio::test]
    async fn miniapp_without_worker_uses_explicit_identity_without_bot_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let mut args = Cli::try_parse_from([
            "market-app",
            "--miniapp-bot-id",
            "42",
            "--miniapp-bot-username",
            "market_bot",
        ])
        .unwrap();
        args.data_dir = directory.path().to_owned();
        let _api = miniapp(
            &args,
            #[cfg(feature = "bot")]
            None,
        )
        .await
        .unwrap();
        assert!(directory.path().join("session.key").is_file());
        assert!(!directory.path().join("bot").exists());
    }

    #[cfg(feature = "bot")]
    #[tokio::test]
    async fn enabled_worker_discovers_identity_and_rejects_manual_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut args = Cli::try_parse_from([
            "market-app",
            "--telegram-bot-token",
            "42:private_test_secret",
            "--telegram-api-id",
            "12345",
            "--telegram-api-hash",
            "0123456789abcdef0123456789abcdef",
        ])
        .unwrap();
        args.data_dir = directory.path().to_owned();
        let mut worker = crate::bot_config::worker(&args).unwrap();
        args.miniapp_bot_id = Some(99);
        assert!(miniapp(&args, worker.as_mut()).await.is_err());
        assert!(!directory.path().join("session.key").exists());
        args.miniapp_bot_id = None;
        args.miniapp_bot_username = Some("wrong_bot".into());
        assert!(miniapp(&args, worker.as_mut()).await.is_err());
        args.miniapp_bot_username = None;
        miniapp(&args, worker.as_mut()).await.unwrap();
        assert!(directory.path().join("session.key").is_file());
        // Configuration does not start Telegram or open its session storage.
        assert!(!directory.path().join("bot").exists());
    }
}

/// Wiring only: API owns authentication; bot owns reports and native prepared messages.
#[cfg(all(feature = "bot", feature = "miniapp"))]
pub fn sharing(bot: market_telegram_bot::Sharing) -> market_api::miniapp::ShareProvider {
    std::sync::Arc::new(move |user, target| {
        let bot = bot.clone();
        Box::pin(async move {
            use market_api::{
                error::{ApiError, ErrorCode},
                miniapp::{ShareTarget, SharedMessage},
            };
            let shared = match target {
                ShareTarget::Gift { slug, currency_id } => bot.gift(user, slug, currency_id).await,
                ShareTarget::Portfolio { query } => bot.portfolio(user, query).await,
                ShareTarget::Market { selection } => bot.market(user, selection).await,
            }
            .map_err(|e| ApiError {
                code: match &e {
                    market_telegram_bot::ShareError::Invalid(_) => ErrorCode::InvalidQuery,
                    market_telegram_bot::ShareError::RestartRead => ErrorCode::RestartRead,
                    market_telegram_bot::ShareError::Busy => ErrorCode::CapacityExceeded,
                    _ => ErrorCode::Unavailable,
                },
                message: e.to_string(),
            })?;
            Ok(SharedMessage {
                id: shared.prepared.map(|p| p.id),
                expires_at: shared.expires_at,
                inline_query: shared.inline_query,
            })
        })
    })
}
