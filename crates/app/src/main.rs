use actix_web::{App, HttpServer, web};
use clap::Parser;
use market_app::{ApiState, cli::Cli};
use std::time::Duration;
#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Cli::parse();
    let node = args.core_client()?;
    let state = ApiState::new(node, args.max_concurrent_queries, args.max_streams);
    let data = web::Data::new(state.clone());
    let origins = args.cors_origin.clone();
    let server = HttpServer::new(move || {
        let mut cors = actix_cors::Cors::default()
            .allowed_methods(vec!["GET", "HEAD"])
            .allowed_origin_fn(|origin, head| {
                head.uri
                    .authority()
                    .map(|v| v.as_str())
                    .or_else(|| head.headers.get("host").and_then(|v| v.to_str().ok()))
                    .is_some_and(|host| {
                        origin.as_bytes() == format!("http://{host}").as_bytes()
                            || origin.as_bytes() == format!("https://{host}").as_bytes()
                    })
            })
            .allowed_headers(vec!["Content-Type"])
            .max_age(3600);
        for origin in &origins {
            cors = cors.allowed_origin(origin);
        }
        App::new()
            .wrap(cors)
            .app_data(web::Data::new(market_app::Origins(origins.clone())))
            .app_data(data.clone())
            .configure(market_app::configure)
    })
    .workers(args.workers)
    .max_connections(1024)
    .client_request_timeout(Duration::from_secs(5))
    .keep_alive(Duration::from_secs(5))
    .shutdown_timeout(5)
    .bind(args.bind)?
    .run();
    tracing::info!(http=%args.bind,"Gateway started; queries are served by core");
    server.await?;
    Ok(())
}
