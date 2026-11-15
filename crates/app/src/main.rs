use actix_web::{App, HttpServer, web};
use clap::Parser;
use market_app::{
    ApiDoc, WebRoot,
    cli::{Cli, Command},
    client::CoreClient,
    configure,
    error::ApiError,
    gateway::Gateway,
};
use utoipa::OpenApi;
#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    if matches!(args.command, Some(Command::Schema)) {
        println!("{}", ApiDoc::openapi().to_json()?);
        return Ok(());
    }
    if matches!(args.command, Some(Command::SessionKey)) {
        println!(
            "{}",
            market_telegram_app::auth::encode_key(&market_telegram_app::auth::generate_key()?)
        );
        return Ok(());
    }
    anyhow::ensure!(
        args.web_dir.join("index.html").is_file(),
        "Build the frontend with npm --prefix web run build, or set --web-dir"
    );
    anyhow::ensure!(
        args.workers > 0 && args.max_streams > 0 && args.max_concurrent_queries > 0,
        "limits must be positive"
    );
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let token = args
        .node_token
        .ok_or_else(|| anyhow::anyhow!("MARKET_NODE_TOKEN is required"))?;
    let identity = token.split_once('.').map(|(id, _)| id).unwrap_or("");
    let source = format!("{}|{}", args.core, identity);
    let node = CoreClient::new(args.core, token)?;
    let data_dir = args.data_dir;
    let key_file = data_dir.join("session.key");
    let replica =
        tokio::task::spawn_blocking(move || market_replica::Replica::open(&data_dir, &source))
            .await??;
    let bot_id = args.telegram_bot_id.filter(|id| *id > 0);
    let key = args.session_key.filter(|v| !v.is_empty());
    let auth = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        bot_id
            .map(|id| {
                market_telegram_app::auth::Auth::production(
                    id,
                    market_telegram_app::auth::session_key(&key_file, key.as_deref())?,
                )
            })
            .transpose()
    })
    .await??;
    let telegram =
        market_app::telegram::Telegram::new(auth, args.telegram_bot_username, args.public_url)?;
    let sync = replica.clone();
    let stop = tokio_util::sync::CancellationToken::new();
    let cancel = stop.clone();
    let worker =
        tokio::spawn(async move { sync.run(node.channel(), node.authorization(), cancel).await });
    let gateway = Gateway::new(replica, args.max_concurrent_queries, args.max_streams);
    let root = WebRoot(args.web_dir);
    let origins = args.cors_origin;
    HttpServer::new(move || {
        let mut cors = actix_cors::Cors::default()
            .allowed_methods(vec!["GET", "POST", "HEAD"])
            .allowed_headers(["Content-Type", "Authorization"]);
        for origin in &origins {
            cors = cors.allowed_origin(origin);
        }
        App::new()
            .wrap(cors)
            .wrap(market_app::security_headers())
            .app_data(
                web::JsonConfig::default()
                    .limit(64 * 1024)
                    .error_handler(|_, _| ApiError::invalid("invalid JSON query").into()),
            )
            .app_data(web::Data::new(telegram.clone()))
            .app_data(web::Data::new(gateway.clone()))
            .app_data(web::Data::new(root.clone()))
            .app_data(web::Data::new(origins.clone()))
            .configure(configure)
    })
    .workers(args.workers)
    .max_connections(2048)
    .client_request_timeout(std::time::Duration::from_secs(10))
    .shutdown_timeout(5)
    .bind(args.bind)?
    .run()
    .await?;
    stop.cancel();
    worker.await?;
    Ok(())
}
