use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};
#[derive(Parser)]
#[command(version, about = "Public market GUI and read-only API")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    #[arg(long, env = "MARKET_HTTP_ADDR", default_value = "127.0.0.1:8080")]
    pub bind: SocketAddr,
    #[arg(long, env = "MARKET_HUB_URL", default_value = "http://127.0.0.1:50051")]
    pub hub_url: String,
    #[arg(long, env = "MARKET_HUB_TOKEN", hide_env_values = true)]
    pub hub_token: Option<String>,
    #[arg(long, env = "MARKET_DATA_DIR", default_value = "data")]
    pub data_dir: PathBuf,
    /// Persistent runtime model media cache; defaults to <data-dir>/media.
    #[arg(long, env = "MARKET_MEDIA_DIR")]
    pub media_dir: Option<PathBuf>,
    #[cfg(feature = "miniapp")]
    #[arg(long, env = "MARKET_MINIAPP_BOT_ID")]
    pub miniapp_bot_id: Option<u64>,
    #[cfg(feature = "miniapp")]
    #[arg(long, env = "MARKET_MINIAPP_BOT_USERNAME")]
    pub miniapp_bot_username: Option<String>,
    #[cfg(any(feature = "bot", feature = "miniapp"))]
    #[arg(long, env = "MARKET_PUBLIC_URL")]
    pub public_url: Option<String>,
    #[cfg(feature = "bot")]
    #[arg(long, env = "TELEGRAM_BOT_TOKEN", hide_env_values = true)]
    pub telegram_bot_token: Option<String>,
    #[cfg(feature = "bot")]
    #[arg(long, visible_alias = "api-id", global = true, env = "TELEGRAM_API_ID")]
    pub telegram_api_id: Option<String>,
    #[cfg(feature = "bot")]
    #[arg(
        long,
        visible_alias = "api-hash",
        global = true,
        env = "TELEGRAM_API_HASH",
        hide_env_values = true
    )]
    pub telegram_api_hash: Option<String>,
    #[cfg(feature = "bot")]
    #[arg(long, env = "MARKET_BOT_DATA_DIR")]
    pub bot_data_dir: Option<PathBuf>,
    /// Authorized user-session export for gift slug lookup; otherwise discover accounts.json.
    #[cfg(feature = "bot")]
    #[arg(long, env = "MARKET_TELEGRAM_ACCOUNTS")]
    pub telegram_accounts: Option<PathBuf>,
    #[cfg(feature = "miniapp")]
    #[arg(long, env = "MARKET_SESSION_KEY", hide_env_values = true)]
    pub session_key: Option<String>,
    #[arg(long, env = "MARKET_WEB_DIR", default_value = "web/dist")]
    pub web_dir: PathBuf,
    /// Run the HTTP API without hosting frontend files.
    #[arg(long, env = "MARKET_NO_WEB", default_value_t = false)]
    pub no_web: bool,
    #[arg(long, env = "MARKET_CORS_ORIGINS", value_delimiter = ',')]
    pub cors_origin: Vec<String>,
    #[arg(long, env = "MARKET_MAX_CONCURRENT_QUERIES", default_value_t = 32)]
    pub max_concurrent_queries: usize,
    #[arg(long, env = "MARKET_MAX_LIVE_QUERIES", default_value_t = 128)]
    pub max_live_queries: usize,
    #[arg(long, env = "MARKET_HTTP_WORKERS", default_value_t = 2)]
    pub http_workers: usize,
}
#[derive(Subcommand)]
pub enum Command {
    /// Print the public HTTP OpenAPI specification.
    Schema,
    /// Print the public WebSocket AsyncAPI specification.
    Asyncapi,
    #[cfg(feature = "miniapp")]
    SessionKey,
    /// Authorize Telegram user sessions for gift lookup.
    #[cfg(feature = "bot")]
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    #[cfg(feature = "bot")]
    Bot {
        #[command(subcommand)]
        command: BotCommand,
    },
}
#[cfg(feature = "bot")]
#[derive(Subcommand)]
pub enum BotCommand {
    Inspect,
    Configure,
}

#[cfg(feature = "bot")]
#[derive(Subcommand)]
pub enum AccountCommand {
    /// Log in by phone/code/2FA and append to accounts.json; no bot token or hub is needed.
    Login {
        /// Export destination; defaults to the configured/discovered file or ./accounts.json.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn telegram_flags_and_commands_follow_compiled_features() {
        let command = Cli::command();
        let has_arg = |id| {
            command
                .get_arguments()
                .any(|argument| argument.get_id() == id)
        };
        assert!(!has_arg("bot_enabled"));
        assert_eq!(has_arg("telegram_bot_token"), cfg!(feature = "bot"));
        assert_eq!(has_arg("telegram_api_id"), cfg!(feature = "bot"));
        assert_eq!(has_arg("telegram_accounts"), cfg!(feature = "bot"));
        assert_eq!(has_arg("telegram_api_hash"), cfg!(feature = "bot"));
        assert_eq!(has_arg("session_key"), cfg!(feature = "miniapp"));
        assert_eq!(has_arg("miniapp_bot_id"), cfg!(feature = "miniapp"));
        assert_eq!(has_arg("miniapp_bot_username"), cfg!(feature = "miniapp"));
        assert!(!has_arg("telegram_bot_id"));
        assert!(!has_arg("telegram_bot_username"));
        assert_eq!(
            has_arg("public_url"),
            cfg!(any(feature = "bot", feature = "miniapp"))
        );
        assert_eq!(
            command.find_subcommand("account").is_some(),
            cfg!(feature = "bot")
        );
        assert_eq!(
            command.find_subcommand("bot").is_some(),
            cfg!(feature = "bot")
        );
        assert_eq!(
            command.find_subcommand("session-key").is_some(),
            cfg!(feature = "miniapp")
        );
    }
    #[test]
    fn deployment_settings_have_environment_bindings_and_typed_cli_overrides() {
        let command = Cli::command();
        for (id, env) in [
            ("hub_url", "MARKET_HUB_URL"),
            ("hub_token", "MARKET_HUB_TOKEN"),
            ("web_dir", "MARKET_WEB_DIR"),
            ("cors_origin", "MARKET_CORS_ORIGINS"),
            ("http_workers", "MARKET_HTTP_WORKERS"),
            ("max_concurrent_queries", "MARKET_MAX_CONCURRENT_QUERIES"),
            ("max_live_queries", "MARKET_MAX_LIVE_QUERIES"),
        ] {
            let arg = command
                .get_arguments()
                .find(|arg| arg.get_id() == id)
                .unwrap();
            assert_eq!(arg.get_env().unwrap(), env);
        }
        let args = Cli::try_parse_from([
            "market-app",
            "--hub-url",
            "http://hub:50051",
            "--hub-token",
            "private",
            "--http-workers",
            "3",
            "--max-concurrent-queries",
            "7",
            "--max-live-queries",
            "9",
            "--cors-origin",
            "https://one.example,https://two.example",
            "--no-web",
        ])
        .unwrap();
        assert_eq!(args.hub_url, "http://hub:50051");
        assert_eq!(args.hub_token.as_deref(), Some("private"));
        assert_eq!(
            (
                args.http_workers,
                args.max_concurrent_queries,
                args.max_live_queries
            ),
            (3, 7, 9)
        );
        assert_eq!(
            args.cors_origin,
            ["https://one.example", "https://two.example"]
        );
        assert!(args.no_web);
        for legacy in [
            "--core",
            "--node-token",
            "--workers",
            "--max-streams",
            "--telegram-bot-id",
            "--bot-enabled",
            "--media-origin",
        ] {
            assert!(Cli::try_parse_from(["market-app", legacy, "1"]).is_err());
        }
    }
}
