//! Service credentials authenticate directly. Errors never trigger enrollment or extra calls.
use hub_protocol::wire as w;
use market_app::client::CoreClient;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_stream::{StreamExt, wrappers::TcpListenerStream};
use tokio_util::sync::CancellationToken;
use tonic::{
    Request, Response, Status,
    transport::{Channel, Server},
};
const TOKEN: &str = "test-node.private-test-service-signature";
#[derive(Clone, Default)]
struct Peer {
    connections: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
    failure: Arc<AtomicUsize>,
}
#[tonic::async_trait]
impl w::market_read_server::MarketRead for Peer {
    async fn list_catalog(
        &self,
        request: Request<w::CatalogQuery>,
    ) -> Result<Response<w::CatalogResult>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            request.metadata().get("authorization").unwrap() == format!("Bearer {TOKEN}").as_str()
        );
        match self.failure.load(Ordering::SeqCst) {
            1 => return Err(Status::permission_denied("namespace denied")),
            2 => return Err(Status::unavailable("retry later")),
            3 => return Err(Status::unauthenticated("signature rejected")),
            _ => {}
        }
        Ok(Response::new(w::CatalogResult {
            meta: Some(w::ReadMeta {
                cursor: Some(w::Cursor {
                    generation: "generation".into(),
                    revision: 0,
                }),
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
    type SyncStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<w::SyncFrame, Status>> + Send>>;
    async fn sync(&self, _: Request<w::SyncRequest>) -> Result<Response<Self::SyncStream>, Status> {
        Err(Status::unimplemented("unused"))
    }
    type ReplayStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<w::SyncFrame, Status>> + Send>>;
    async fn replay(
        &self,
        _: Request<w::SyncRequest>,
    ) -> Result<Response<Self::ReplayStream>, Status> {
        Err(Status::unimplemented("unused"))
    }
}
async fn catalog(node: &CoreClient) -> Result<w::CatalogResult, Status> {
    node.authenticated(|channel: Channel, auth| async move {
        let mut rpc = w::market_read_client::MarketReadClient::new(channel);
        let mut request = Request::new(w::CatalogQuery::default());
        request.metadata_mut().insert("authorization", auth);
        request.set_timeout(std::time::Duration::from_secs(2));
        Ok(rpc.list_catalog(request).await?.into_inner())
    })
    .await
}
fn node(endpoint: String) -> CoreClient {
    CoreClient::new(endpoint, TOKEN.into()).unwrap()
}
#[tokio::test]
async fn service_tokens_reuse_one_channel_and_errors_never_trigger_extra_calls() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = Peer::default();
    let service = peer.clone();
    let connections = peer.connections.clone();
    let node = node(format!("http://{}", listener.local_addr().unwrap()));
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(service))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener).map(move |c| {
                    if c.is_ok() {
                        connections.fetch_add(1, Ordering::SeqCst);
                    }
                    c
                }),
                cancel.cancelled_owned(),
            )
            .await
            .unwrap();
    });
    for _ in 0..4 {
        catalog(&node.clone()).await.unwrap();
    }
    assert_eq!(peer.connections.load(Ordering::SeqCst), 1);
    for (failure, code) in [
        (1, tonic::Code::PermissionDenied),
        (2, tonic::Code::Unavailable),
        (3, tonic::Code::Unauthenticated),
    ] {
        let before = peer.calls.load(Ordering::SeqCst);
        peer.failure.store(failure, Ordering::SeqCst);
        assert_eq!(catalog(&node).await.unwrap_err().code(), code);
        assert_eq!(peer.calls.load(Ordering::SeqCst), before + 1);
    }
    peer.failure.store(0, Ordering::SeqCst);
    catalog(&node).await.unwrap();
    assert_eq!(peer.connections.load(Ordering::SeqCst), 1);
    stop.cancel();
    server.await.unwrap();
}
#[tokio::test]
async fn uri_and_token_validation_support_remote_http_and_reject_invalid_inputs() {
    assert!(CoreClient::new("http://hub.example:50051".into(), TOKEN.into()).is_ok());
    for endpoint in [
        "https://hub.example:50051",
        "http://user@hub.example",
        "http://hub.example/token",
        "http://hub.example?token=secret",
    ] {
        assert!(CoreClient::new(endpoint.into(), TOKEN.into()).is_err());
    }
    for token in [
        "invalid",
        ".signature-is-empty",
        "service.",
        "bad:id.signature",
        "node.white space",
    ] {
        assert!(CoreClient::new("http://127.0.0.1:50051".into(), token.into()).is_err());
    }
}
