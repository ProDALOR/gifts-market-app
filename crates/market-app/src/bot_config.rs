//! Application wiring only; bot transport, commands and MTProto updates live in the Telegram crate.
use crate::cli::{BotCommand, Cli};
use market_telegram::bot::{self, BotConfig, BotCredentials, BotWorker};

fn credentials(args: &Cli) -> anyhow::Result<BotCredentials> {
    BotCredentials::new(
        args.telegram_bot_token
            .clone()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("TELEGRAM_BOT_TOKEN is required when the bot worker is enabled")
            })?,
        args.telegram_api_id.ok_or_else(|| {
            anyhow::anyhow!("TELEGRAM_API_ID is required when the bot worker is enabled")
        })?,
        args.telegram_api_hash.clone().ok_or_else(|| {
            anyhow::anyhow!("TELEGRAM_API_HASH is required when the bot worker is enabled")
        })?,
    )
}
fn data_dir(args: &Cli) -> std::path::PathBuf {
    args.bot_data_dir
        .clone()
        .unwrap_or_else(|| args.data_dir.join("bot"))
}
fn config(args: &Cli) -> anyhow::Result<BotConfig> {
    Ok(BotConfig {
        credentials: credentials(args)?,
        public_url: args.public_url.clone().filter(|url| !url.is_empty()),
        data_dir: data_dir(args),
    })
}
pub fn worker(args: &Cli) -> anyhow::Result<Option<BotWorker>> {
    if args.bot_enabled {
        Ok(Some(BotWorker::new(config(args)?)?))
    } else {
        Ok(None)
    }
}
pub async fn manage(args: &Cli, command: &BotCommand) -> anyhow::Result<()> {
    match command {
        BotCommand::Inspect => {
            let inspection = bot::inspect(credentials(args)?, data_dir(args)).await?;
            println!(
                "@{} (ID {})\nMain Mini App: {}\nMTProto session: {}",
                inspection.user.username,
                inspection.user.id,
                inspection.user.has_main_web_app,
                inspection.session_path.display()
            );
        }
        BotCommand::Configure => {
            let user = bot::configure(config(args)?).await?;
            println!("Configured bot commands for @{}", user.username);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn disabled_worker_requires_no_bot_credentials_and_enabled_worker_validates_configuration() {
        let mut args = Cli::try_parse_from(["market-app", "--bot-enabled", "false"]).unwrap();
        args.telegram_bot_token = None;
        args.public_url = None;
        assert!(worker(&args).unwrap().is_none());
        args.bot_enabled = true;
        assert!(worker(&args).is_err());
        args.telegram_bot_token = Some("42:private_test_secret".into());
        args.telegram_api_id = Some(12345);
        args.telegram_api_hash = Some("0123456789abcdef0123456789abcdef".into());
        assert!(worker(&args).unwrap().is_some());
        args.public_url = Some("https://market.example".into());
        assert!(worker(&args).unwrap().is_some());
        args.public_url = Some("http://market.example".into());
        assert!(worker(&args).is_err());
    }
}
