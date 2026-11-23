//! Assemble Telegram transport; market commands and bot service live in market-telegram-bot.
use crate::cli::{BotCommand, Cli};
use market_telegram::{BotConfig, BotCredentials, TelegramWorker};

fn credentials(args: &Cli) -> anyhow::Result<Option<BotCredentials>> {
    let fields = [
        ("TELEGRAM_BOT_TOKEN", args.telegram_bot_token.as_deref()),
        ("TELEGRAM_API_ID", args.telegram_api_id.as_deref()),
        ("TELEGRAM_API_HASH", args.telegram_api_hash.as_deref()),
    ]
    .map(|(name, value)| (name, value.map(str::trim).filter(|value| !value.is_empty())));
    if fields.iter().all(|(_, value)| value.is_none()) {
        return Ok(None);
    }
    let [(_, token), (_, api_id), (_, api_hash)] = fields;
    let (Some(token), Some(api_id), Some(api_hash)) = (token, api_id, api_hash) else {
        let missing: Vec<_> = fields
            .iter()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| *name)
            .collect();
        anyhow::bail!(
            "Incomplete Telegram bot credentials: missing {}",
            missing.join(", ")
        );
    };
    let api_id = api_id
        .parse::<i32>()
        .map_err(|_| anyhow::anyhow!("TELEGRAM_API_ID must be a positive integer"))?;
    Ok(Some(BotCredentials::new(
        token.to_owned(),
        api_id,
        api_hash.to_owned(),
    )?))
}
fn data_dir(args: &Cli) -> std::path::PathBuf {
    args.bot_data_dir
        .clone()
        .unwrap_or_else(|| args.data_dir.join("bot"))
}
fn config(args: &Cli) -> anyhow::Result<Option<BotConfig>> {
    Ok(credentials(args)?.map(|credentials| BotConfig {
        credentials,
        public_url: args.public_url.clone().filter(|url| !url.is_empty()),
        data_dir: data_dir(args),
    }))
}
pub fn worker(args: &Cli) -> anyhow::Result<Option<TelegramWorker>> {
    config(args)?.map(TelegramWorker::new).transpose()
}
pub async fn manage(args: &Cli, command: &BotCommand) -> anyhow::Result<()> {
    let config = config(args)?.ok_or_else(|| anyhow::anyhow!("Configure TELEGRAM_BOT_TOKEN, TELEGRAM_API_ID and TELEGRAM_API_HASH to use bot commands"))?;
    match command {
        BotCommand::Inspect => {
            let inspection = market_telegram::inspect(config.credentials, config.data_dir).await?;
            println!(
                "@{} (ID {})\nMain Mini App: {}\nInline mode: {}\nGuest mode: {}\nMTProto session: {}",
                inspection.user.username,
                inspection.user.id,
                inspection.user.has_main_web_app,
                inspection.user.inline_enabled,
                inspection.user.guest_enabled,
                inspection.session_path.display()
            );
        }
        BotCommand::Configure => {
            let user = market_telegram_bot::configure(config).await?;
            println!("Configured bot commands for @{}", user.username);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    fn args() -> Cli {
        let mut args = Cli::try_parse_from(["market-app"]).unwrap();
        args.telegram_bot_token = None;
        args.telegram_api_id = None;
        args.telegram_api_hash = None;
        args.public_url = None;
        args
    }
    #[test]
    fn absent_or_blank_credentials_leave_worker_disabled() {
        let mut args = args();
        assert!(worker(&args).unwrap().is_none());
        args.telegram_bot_token = Some("  ".into());
        args.telegram_api_id = Some(String::new());
        args.telegram_api_hash = Some("\t".into());
        assert!(worker(&args).unwrap().is_none());
    }
    #[test]
    fn every_partial_credential_combination_reports_missing_fields_without_secrets() {
        for mask in 1..7 {
            let mut args = args();
            args.telegram_bot_token = (mask & 1 != 0).then(|| "42:private_test_secret".into());
            args.telegram_api_id = (mask & 2 != 0).then(|| "12345".into());
            args.telegram_api_hash =
                (mask & 4 != 0).then(|| "0123456789abcdef0123456789abcdef".into());
            let error = worker(&args).err().unwrap().to_string();
            assert!(error.contains("Incomplete Telegram bot credentials"));
            for (bit, name) in [
                (1, "TELEGRAM_BOT_TOKEN"),
                (2, "TELEGRAM_API_ID"),
                (4, "TELEGRAM_API_HASH"),
            ] {
                assert_eq!(error.contains(name), mask & bit == 0);
            }
            assert!(!error.contains("private_test_secret"));
            assert!(!error.contains("0123456789abcdef0123456789abcdef"));
        }
    }
    #[test]
    fn complete_credentials_start_worker_and_validate_values_and_launch_url() {
        let mut args = args();
        args.telegram_bot_token = Some("42:private_test_secret".into());
        args.telegram_api_id = Some("12345".into());
        args.telegram_api_hash = Some("0123456789abcdef0123456789abcdef".into());
        assert!(worker(&args).unwrap().is_some());
        args.public_url = Some("https://market.example".into());
        assert!(worker(&args).unwrap().is_some());
        args.public_url = Some("http://market.example".into());
        assert!(worker(&args).is_err());
        args.public_url = None;
        for invalid in ["not-an-id", "0", "-1", "2147483648"] {
            args.telegram_api_id = Some(invalid.into());
            assert!(
                worker(&args)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("TELEGRAM_API_ID")
            );
        }
        args.telegram_api_id = Some(" 12345 ".into());
        assert!(worker(&args).unwrap().is_some());
    }
}
