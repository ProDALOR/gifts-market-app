use clap::Parser;
use market_app::{
    MarketRuntime,
    cli::{Cli, Command},
};
use market_hub_client::HubClient;

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    match &args.command {
        Some(Command::Schema) => {
            println!("{}", market_api::schema_json()?);
            return Ok(());
        }
        Some(Command::Asyncapi) => {
            println!("{}", market_api::asyncapi_json()?);
            return Ok(());
        }
        #[cfg(feature = "miniapp")]
        Some(Command::SessionKey) => {
            println!(
                "{}",
                market_miniapp::auth::encode_key(&market_miniapp::auth::generate_key()?)
            );
            return Ok(());
        }
        #[cfg(feature = "bot")]
        Some(Command::Bot { command }) => {
            return market_app::bot_config::manage(&args, command).await;
        }
        None => {}
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    #[cfg(feature = "bot")]
    let mut bot = market_app::bot_config::worker(&args)?;
    let token = args
        .hub_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("MARKET_HUB_TOKEN is required"))?;
    let hub = HubClient::new(args.hub_url.clone(), token)?;
    let runtime = MarketRuntime::open(
        &args.data_dir,
        hub.source(),
        args.max_concurrent_queries,
        args.max_live_queries,
    )
    .await?;
    let media_dir = args
        .media_dir
        .clone()
        .unwrap_or_else(|| args.data_dir.join("media"));
    let (media, downloads) = market_media::MediaCache::open(&media_dir).await?;
    #[cfg(feature = "bot")]
    if let Some(bot) = bot.as_mut() {
        bot.set_media_downloader(downloads);
    } else {
        drop(downloads);
    }
    #[cfg(not(feature = "bot"))]
    drop(downloads);

    let api = market_app::api_config::server(
        &args,
        runtime.handle(),
        runtime.valuation(),
        #[cfg(feature = "bot")]
        bot.as_mut(),
    )
    .await?
    .with_media(media);
    runtime.start_replication(hub)?;
    #[cfg(feature = "bot")]
    if let Some(bot) = bot {
        runtime.start_bot(bot)?;
    }
    runtime.start_api(api)?;
    let signal = tokio::select! {
        signal = shutdown_signal() => signal,
        _ = runtime.cancelled() => Ok(()),
    };
    let shutdown = runtime.shutdown().await;
    signal?;
    shutdown
}
