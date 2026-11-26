mod startup;
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
        Some(Command::Account { command }) => {
            return market_app::account_login::manage(&args, command).await;
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
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "market app starting");
    let stop = tokio_util::sync::CancellationToken::new();
    startup::until_shutdown(stop.clone(), serve(args, stop), shutdown_signal()).await
}
async fn serve(args: Cli, stop: tokio_util::sync::CancellationToken) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    #[cfg(feature = "bot")]
    let mut bot = market_app::bot_config::worker(&args)?;
    let token = args
        .hub_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("MARKET_HUB_TOKEN is required"))?;
    let hub = HubClient::new(args.hub_url.clone(), token)?;
    let runtime = startup::phase(
        "market_runtime",
        &stop,
        MarketRuntime::open_with_cancellation(
            &args.data_dir,
            hub.source(),
            args.max_concurrent_queries,
            args.max_live_queries,
            stop.clone(),
        ),
    )
    .await;
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(_) if stop.is_cancelled() => return Ok(()),
        Err(error) => return Err(error),
    };
    let service_stop = runtime.cancellation_token();
    let mut media_cache = None;
    let result = async {
        #[cfg(feature = "bot")]
        if let Some(accounts) = startup::phase(
            "telegram_accounts",
            &service_stop,
            market_app::account_config::worker(&args),
        )
        .await?
        {
            runtime.start_accounts(accounts)?;
        }
        #[cfg(feature = "bot")]
        if let Some(bot) = bot.as_ref() {
            runtime.start_portfolio(bot.inventory()).await?;
        }
        let media_dir = args
            .media_dir
            .clone()
            .unwrap_or_else(|| args.data_dir.join("media"));
        let (media, downloads) = startup::phase(
            "media_cache",
            &service_stop,
            market_media::MediaCache::open_with_cancellation(&media_dir, service_stop.clone()),
        )
        .await?;
        media_cache = Some(media.clone());
        #[cfg(feature = "bot")]
        if let Some(bot) = bot.as_mut() {
            bot.set_media(media.clone(), downloads);
        } else {
            drop(downloads);
        }
        #[cfg(not(feature = "bot"))]
        drop(downloads);

        let api = startup::phase(
            "http_api",
            &service_stop,
            market_app::api_config::server(
                &args,
                runtime.handle(),
                runtime.valuation(),
                #[cfg(feature = "bot")]
                bot.as_mut(),
            ),
        )
        .await?
        .with_media(media);
        #[cfg(feature = "bot")]
        let api = if let Some(portfolio) = runtime.portfolio() {
            api.with_portfolio(portfolio)
        } else {
            api
        };
        runtime.start_replication(hub)?;
        #[cfg(feature = "bot")]
        let api = if let Some(bot) = bot {
            let sharing = runtime.start_bot(bot)?;
            #[cfg(feature = "miniapp")]
            let api = api.with_sharing(market_app::api_config::sharing(sharing));
            #[cfg(not(feature = "miniapp"))]
            let _ = sharing;
            api
        } else {
            api
        };
        runtime.start_api(api)?;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            "market app services started"
        );
        runtime.cancelled().await;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let shutdown = runtime.shutdown().await;
    if let Some(media) = media_cache {
        media.shutdown().await;
    }
    shutdown?;
    if stop.is_cancelled() { Ok(()) } else { result }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn startup_failure_drains_services_and_releases_the_replica_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mut args = Cli::try_parse_from(["market-app"]).unwrap();
        args.data_dir = dir.path().join("data");
        args.media_dir = Some(dir.path().join("media"));
        args.hub_url = "http://127.0.0.1:1".into();
        args.hub_token = Some("test.0123456789abcdef".into());
        args.bind = "127.0.0.1:0".parse().unwrap();
        args.no_web = false;
        args.web_dir = dir.path().join("missing-web");
        #[cfg(feature = "bot")]
        {
            args.telegram_bot_token = None;
            args.telegram_api_id = None;
            args.telegram_api_hash = None;
            args.telegram_accounts = Some(dir.path().join("missing-accounts.json"));
            args.bot_data_dir = Some(dir.path().join("bot"));
        }
        #[cfg(feature = "miniapp")]
        {
            args.miniapp_bot_id = None;
            args.miniapp_bot_username = None;
        }
        let source = HubClient::new(args.hub_url.clone(), "test.0123456789abcdef".into())
            .unwrap()
            .source()
            .to_owned();
        let path = args.data_dir.clone();
        assert!(
            serve(args, tokio_util::sync::CancellationToken::new())
                .await
                .is_err()
        );
        let reopened = market_replica::ReplicaWriter::open(&path, &source).unwrap();
        assert!(reopened.cursor().unwrap().is_empty());
    }
}
