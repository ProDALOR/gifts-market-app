//! Bound service ownership and orderly draining of HTTP, WebSocket and encoding work.
#[cfg(feature = "miniapp")]
use crate::miniapp::MiniAppApi;
use crate::{
    MarketApi,
    config::ApiConfig,
    connections::ConnectionSupervisor,
    frontend::{self, WebRoot},
    origin::OriginPolicy,
    security_headers,
    websocket::WebSocketService,
};
use actix_web::{App, HttpServer, dev::ServerHandle, middleware::ErrorHandlers, web};
use market_actors::MarketDataHandle;
use std::{
    io,
    net::{SocketAddr, TcpListener},
};
use tokio_util::sync::CancellationToken;
pub struct ApiServer {
    config: ApiConfig,
    listener: TcpListener,
    data: MarketDataHandle,
    origins: OriginPolicy,
    #[cfg(feature = "miniapp")]
    miniapp: Option<MiniAppApi>,
}
impl ApiServer {
    pub fn bind(config: ApiConfig, data: MarketDataHandle) -> anyhow::Result<Self> {
        anyhow::ensure!(config.workers > 0, "API worker count must be positive");
        config.limits.validate()?;
        let origins = OriginPolicy::new(&config.cors_origins)?;
        if let Some(directory) = &config.web_dir {
            anyhow::ensure!(
                directory.join("index.html").is_file(),
                "Build the frontend, set --web-dir, or use --no-web"
            );
        }
        let listener = TcpListener::bind(config.bind)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            config,
            listener,
            data,
            origins,
            #[cfg(feature = "miniapp")]
            miniapp: None,
        })
    }
    #[cfg(feature = "miniapp")]
    pub fn with_miniapp(mut self, miniapp: MiniAppApi) -> Self {
        self.miniapp = Some(miniapp);
        self
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    pub async fn run(self, stop: CancellationToken) -> io::Result<()> {
        if stop.is_cancelled() {
            return Ok(());
        }
        let Self {
            config,
            listener,
            data,
            origins,
            #[cfg(feature = "miniapp")]
            miniapp,
        } = self;
        let connections = ConnectionSupervisor::with_limit(
            stop.child_token(),
            config.limits.websocket.connections,
        );
        let market = MarketApi::with_page_limit(data, config.limits.websocket.page_bytes);
        let ws = web::Data::new(WebSocketService {
            market: market.clone(),
            connections: connections.clone(),
            origins: origins.clone(),
            limits: config.limits.websocket.clone(),
        });
        let state = web::Data::new(market.clone());
        let root = config.web_dir.map(|path| web::Data::new(WebRoot(path)));
        let json_limit = config.limits.json_bytes;
        let server = HttpServer::new(move || {
            let app = App::new()
                .wrap(origins.cors())
                .wrap(
                    ErrorHandlers::new()
                        .default_handler_client(crate::error::normalize)
                        .default_handler_server(crate::error::normalize),
                )
                .wrap(security_headers())
                .app_data(state.clone())
                .app_data(ws.clone())
                .configure(|cfg| crate::routes::configure_with_limit(cfg, json_limit));
            let app = if let Some(root) = &root {
                app.app_data(root.clone()).configure(frontend::configure)
            } else {
                app
            };
            #[cfg(feature = "miniapp")]
            let app = if let Some(miniapp) = &miniapp {
                app.app_data(web::Data::new(miniapp.clone()))
            } else {
                app
            };
            app
        })
        .workers(config.workers)
        .max_connections(config.limits.http_connections)
        .client_request_timeout(config.limits.request_timeout)
        .shutdown_timeout(5)
        .disable_signals()
        .listen(listener)?
        .run();
        let mut guard = ServerGuard {
            handle: Some(server.handle()),
            connections,
            market,
        };
        tokio::pin!(server);
        let result = tokio::select! {
            result=&mut server=>result,
            _=stop.cancelled()=>{
                guard.connections.begin_shutdown();
                let (_,result)=tokio::join!(guard.handle.as_ref().unwrap().stop(true),&mut server);result
            }
        };
        guard.connections.shutdown().await;
        guard.market.shutdown().await;
        guard.handle = None;
        result
    }
}
struct ServerGuard {
    handle: Option<ServerHandle>,
    connections: ConnectionSupervisor,
    market: MarketApi,
}
impl Drop for ServerGuard {
    fn drop(&mut self) {
        self.connections.begin_shutdown();
        if let Some(handle) = self.handle.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let connections = self.connections.clone();
            let market = self.market.clone();
            runtime.spawn(async move {
                tokio::join!(handle.stop(false), async {
                    connections.shutdown().await;
                    market.shutdown().await;
                });
            });
        }
    }
}
