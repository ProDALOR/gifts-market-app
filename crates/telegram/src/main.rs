use clap::{Parser, Subcommand};
use market_telegram_app::bot::{self, Bot, Checkpoint};
use std::path::PathBuf;
#[derive(Parser)]
#[command(about = "Telegram launch bot for the public market app", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(long, env = "TELEGRAM_BOT_TOKEN", hide_env_values = true)]
    token: Option<String>,
    #[arg(long, env = "MARKET_PUBLIC_URL")]
    app_url: Option<String>,
    #[arg(long, env = "MARKET_BOT_DATA_DIR", default_value = "bot-data")]
    data_dir: PathBuf,
}
#[derive(Subcommand)]
enum Command {
    Inspect,
    Configure,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let args = Cli::parse();
    let bot = Bot::new(
        &args
            .token
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("TELEGRAM_BOT_TOKEN is required"))?,
    )?;
    if matches!(args.command, Some(Command::Inspect)) {
        let user = bot.me().await?;
        let webhook = bot.webhook().await?;
        println!(
            "@{} (ID {})\nMain Mini App: {}\nWebhook configured: {}",
            user.username,
            user.id,
            user.has_main_web_app,
            !webhook.url.is_empty()
        );
        return Ok(());
    }
    let url = bot::app_url(&args.app_url.ok_or_else(|| {
        anyhow::anyhow!(
            "MARKET_PUBLIC_URL is required: deploy the app or use an HTTPS tunnel first"
        )
    })?)?;
    let user = bot.me().await?;
    let checkpoint = Checkpoint::open(&args.data_dir, user.id)?;
    if args.command.is_none() {
        anyhow::ensure!(
            bot.webhook().await?.url.is_empty(),
            "This bot already has a webhook; remove it explicitly before starting polling"
        );
    }
    bot.configure(&url).await?;
    println!("Configured launch buttons for @{}", user.username);
    if matches!(args.command, Some(Command::Configure)) {
        return Ok(());
    }
    let stop = tokio_util::sync::CancellationToken::new();
    let cancel = stop.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        cancel.cancel();
    });
    bot::run(bot, user, url, checkpoint, stop).await
}
