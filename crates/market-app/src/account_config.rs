//! Optional user-session imports. Session secrets remain in the original export.
use crate::cli::Cli;
use market_telegram_client::AccountWorker;
pub async fn worker(args: &Cli) -> anyhow::Result<Option<AccountWorker>> {
    let directory = args
        .bot_data_dir
        .clone()
        .unwrap_or_else(|| args.data_dir.join("bot"));
    let path = if let Some(path) = &args.telegram_accounts {
        path.clone()
    } else if tokio::fs::try_exists("accounts.json").await? {
        "accounts.json".into()
    } else {
        let path = directory.join("accounts.json");
        if !tokio::fs::try_exists(&path).await? {
            return Ok(None);
        }
        path
    };
    Ok(Some(
        AccountWorker::from_file(&path, directory.join("accounts")).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[tokio::test]
    async fn explicit_account_exports_need_no_bot_credentials_and_invalid_paths_fail_safely() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mut args = Cli::try_parse_from(["market-app"]).unwrap();
        args.telegram_accounts = Some(path.clone());
        args.telegram_bot_token = None;
        args.telegram_api_id = None;
        args.telegram_api_hash = None;
        args.bot_data_dir = Some(dir.path().join("bot"));
        let Err(error) = worker(&args).await else {
            panic!("missing explicit export accepted")
        };
        assert!(
            error
                .to_string()
                .contains("Cannot read Telegram accounts file")
        );
        std::fs::write(&path,serde_json::to_vec(&serde_json::json!([{"api_id":12345,"auth_key":"ab".repeat(256),"dc_id":2,"server_address":"127.0.0.1:443"}])).unwrap()).unwrap();
        assert!(crate::bot_config::worker(&args).unwrap().is_none());
        let accounts = worker(&args).await.unwrap().unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        accounts.run(stop).await.unwrap();
        assert!(!dir.path().join("bot").exists());
    }
}
