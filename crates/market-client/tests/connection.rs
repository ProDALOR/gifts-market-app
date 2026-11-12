//! Exercise enrollment using a protocol peer, including lost replies and transport reuse.
use hub_protocol::wire as w;
use market_client::{node::NodeConfig, registration::NodeCredentials};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio_stream::{StreamExt, wrappers::TcpListenerStream};
use tokio_util::sync::CancellationToken;
use tonic::{
    Request, Response, Status,
    transport::{Channel, Server},
};

const JOIN: &str = "test-deployment-join-credential";
const TOKEN: &str = "test-independent-node-credential";
#[derive(Clone, Default)]
struct Peer {
    registered: Arc<AtomicBool>,
    registrations: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
    failure: Arc<AtomicUsize>,
    lose_reply: Arc<AtomicBool>,
}
#[tonic::async_trait]
impl w::market_registration_server::MarketRegistration for Peer {
    async fn register_node(
        &self,
        request: Request<w::RegisterNodeRequest>,
    ) -> Result<Response<w::Empty>, Status> {
        self.registrations.fetch_add(1, Ordering::SeqCst);
        assert!(
            request.metadata().get("authorization").unwrap() == format!("Bearer {JOIN}").as_str()
        );
        let request = request.into_inner();
        assert!(request.node_token == TOKEN && request.node_token != JOIN);
        self.registered.store(true, Ordering::SeqCst);
        if self.lose_reply.swap(false, Ordering::SeqCst) {
            return Err(Status::unavailable("registered, but reply lost"));
        }
        Ok(Response::new(w::Empty {}))
    }
}
#[tonic::async_trait]
impl w::market_read_server::MarketRead for Peer {
    async fn list_catalog(
        &self,
        request: Request<w::CatalogQuery>,
    ) -> Result<Response<w::CatalogResult>, Status> {
        match self.failure.load(Ordering::SeqCst) {
            1 => return Err(Status::permission_denied("wrong role")),
            2 => return Err(Status::unavailable("retry later")),
            _ => {}
        }
        if !self.registered.load(Ordering::SeqCst) {
            return Err(Status::unauthenticated("unknown node"));
        }
        assert!(
            request.metadata().get("authorization").unwrap() == format!("Bearer {TOKEN}").as_str()
        );
        Ok(Response::new(w::CatalogResult {
            meta: Some(w::ReadMeta {
                cursor: Some(w::Cursor {
                    generation: "generation".into(),
                    revision: 0,
                }),
                catalog_revision: "revision".into(),
                ..Default::default()
            }),
            ..Default::default()
        }))
    }
    async fn get_status(&self, _: Request<w::Empty>) -> Result<Response<w::ReadStatus>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn list_feeds(
        &self,
        _: Request<w::FeedsQuery>,
    ) -> Result<Response<w::FeedsResult>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn list_items(
        &self,
        _: Request<w::ItemsQuery>,
    ) -> Result<Response<w::ItemsResult>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn list_orders(
        &self,
        _: Request<w::OrdersQuery>,
    ) -> Result<Response<w::OrdersResult>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn list_stored_orders(
        &self,
        _: Request<w::OrdersQuery>,
    ) -> Result<Response<w::OrdersResult>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn get_order_book(
        &self,
        _: Request<w::BookQuery>,
    ) -> Result<Response<w::BookResult>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn list_trades(
        &self,
        _: Request<w::TradesQuery>,
    ) -> Result<Response<w::TradesResult>, Status> {
        Err(Status::unimplemented("unused"))
    }
    type WatchOrderBookStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<w::BookResult, Status>> + Send>>;
    async fn watch_order_book(
        &self,
        _: Request<w::BookQuery>,
    ) -> Result<Response<Self::WatchOrderBookStream>, Status> {
        Err(Status::unimplemented("unused"))
    }
    type ReplayStream = Self::SyncStream;
    async fn replay(
        &self,
        _: Request<w::SyncRequest>,
    ) -> Result<Response<Self::ReplayStream>, Status> {
        Err(Status::unimplemented("unused"))
    }
    type SyncStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<w::SyncFrame, Status>> + Send>>;
    async fn sync(&self, _: Request<w::SyncRequest>) -> Result<Response<Self::SyncStream>, Status> {
        Err(Status::unimplemented("unused"))
    }
}
async fn catalog(node: &NodeConfig) -> Result<w::CatalogResult, Status> {
    node.authenticated(|channel: Channel, auth| async move {
        let mut rpc = w::market_read_client::MarketReadClient::new(channel);
        let mut request = Request::new(w::CatalogQuery::default());
        request.metadata_mut().insert("authorization", auth);
        request.set_timeout(std::time::Duration::from_secs(2));
        Ok(rpc.list_catalog(request).await?.into_inner())
    })
    .await
}
fn node(endpoint: String) -> NodeConfig {
    NodeConfig::new(
        endpoint,
        NodeCredentials {
            id: "test-node".into(),
            token: TOKEN.into(),
        },
    )
}
#[tokio::test]
async fn enrollment_is_on_demand_and_one_channel_survives_retries_and_clones() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = Peer::default();
    let service = peer.clone();
    let connections = peer.connections.clone();
    let mut node = node(format!("http://{}", listener.local_addr().unwrap()));
    node.join_token = Some(JOIN.into());
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(
                w::market_registration_server::MarketRegistrationServer::new(service.clone()),
            )
            .add_service(w::market_read_server::MarketReadServer::new(service))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener).map(move |connection| {
                    if connection.is_ok() {
                        connections.fetch_add(1, Ordering::SeqCst);
                    }
                    connection
                }),
                cancel.cancelled_owned(),
            )
            .await
            .unwrap();
    });
    let channel = node.connect().await.unwrap();
    assert_eq!(peer.registrations.load(Ordering::SeqCst), 0);
    catalog(&node).await.unwrap();
    assert_eq!(peer.registrations.load(Ordering::SeqCst), 1);
    // Enrollment token rotation or omission must not affect an established identity.
    let mut existing = node.clone();
    existing.join_token = Some("old-or-rotated-join-credential".into());
    for _ in 0..3 {
        catalog(&existing).await.unwrap();
    }
    existing.join_token = None;
    catalog(&existing).await.unwrap();
    assert_eq!(peer.registrations.load(Ordering::SeqCst), 1);
    assert_eq!(peer.connections.load(Ordering::SeqCst), 1);
    // A cleared registry can be restored without generating a different node token.
    peer.registered.store(false, Ordering::SeqCst);
    assert_eq!(
        catalog(&existing).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    catalog(&node).await.unwrap();
    assert_eq!(peer.registrations.load(Ordering::SeqCst), 2);
    // Neither service authorization nor network failures trigger enrollment.
    for (failure, code) in [
        (1, tonic::Code::PermissionDenied),
        (2, tonic::Code::Unavailable),
    ] {
        peer.failure.store(failure, Ordering::SeqCst);
        assert_eq!(catalog(&node).await.unwrap_err().code(), code);
    }
    assert_eq!(peer.registrations.load(Ordering::SeqCst), 2);
    peer.failure.store(0, Ordering::SeqCst);
    // Lost enrollment acknowledgements recover by using the already saved credential.
    peer.registered.store(false, Ordering::SeqCst);
    peer.lose_reply.store(true, Ordering::SeqCst);
    assert_eq!(
        catalog(&node).await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
    catalog(&node).await.unwrap();
    assert_eq!(peer.registrations.load(Ordering::SeqCst), 3);
    assert_eq!(peer.connections.load(Ordering::SeqCst), 1);
    drop(channel);
    stop.cancel();
    server.await.unwrap();
}
#[tokio::test]
async fn uri_and_token_validation_do_not_require_private_ca_for_public_https() {
    node("https://hub.example:50051".into()).validate().unwrap();
    for endpoint in [
        "http://hub.example:50051",
        "https://user@hub.example",
        "https://hub.example/token",
        "https://hub.example?token=secret",
    ] {
        assert!(node(endpoint.into()).validate().is_err());
    }
    let mut node = node("http://127.0.0.1:50051".into());
    node.join_token = Some(TOKEN.into());
    assert!(node.validate().is_err());
}
