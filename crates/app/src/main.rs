use actix_web::{App, HttpServer, web};
use clap::Parser;
use market_app::{ApiState, cli::Cli, replication::ReplicaHandle};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Cli::parse();
    let config = args.replica_config()?;
    let tls = if let (Some(cert), Some(key)) = (&args.grpc_cert, &args.grpc_key) {
        let mut tls = tonic::transport::ServerTlsConfig::new().identity(
            tonic::transport::Identity::from_pem(
                tokio::fs::read(cert).await?,
                tokio::fs::read(key).await?,
            ),
        );
        if let Some(ca) = &args.grpc_client_ca {
            tls = tls.client_ca_root(tonic::transport::Certificate::from_pem(
                tokio::fs::read(ca).await?,
            ));
        }
        Some(tls)
    } else {
        None
    };
    let grpc_listener = if let Some(bind) = args.grpc_bind {
        Some(tokio::net::TcpListener::bind(bind).await?)
    } else {
        None
    };
    let replica = ReplicaHandle::with_replay_limits(args.replay_batches, args.replay_bytes);
    let state = ApiState::new(
        replica.clone(),
        args.max_stale,
        args.max_concurrent_queries,
        args.max_scan,
    )
    .limits(
        args.max_streams,
        args.max_snapshots,
        args.max_snapshot_bytes,
        args.snapshot_ttl,
    );
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
    let server_handle = server.handle();
    let stop = CancellationToken::new();
    let shutdown = stop.clone();
    let sync = tokio::spawn(async move { replica.run(config, shutdown).await });
    let shutdown = stop.clone();
    let mut grpc = tokio::spawn(async move {
        if let Some(listener) = grpc_listener {
            market_app::grpc::serve(state, listener, tls, args.grpc_token, shutdown).await
        } else {
            shutdown.cancelled().await;
            Ok(())
        }
    });
    tracing::info!(http=%args.bind,grpc=?args.grpc_bind,"API node started; waiting for local bootstrap");
    let result = tokio::select! {
        result=server=>result.map_err(anyhow::Error::from),
        result=&mut grpc=>{
            stop.cancel(); server_handle.stop(true).await;
            match result {Ok(Ok(()))=>Err(anyhow::anyhow!("gRPC listener stopped unexpectedly")),Ok(Err(e))=>Err(e),Err(e)=>Err(e.into())}
        }
    };
    stop.cancel();
    if !grpc.is_finished() {
        let _ = tokio::time::timeout(Duration::from_secs(5), grpc).await;
    }
    let _ = sync.await;
    result
}
