use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_stream::{StreamExt, wrappers::TcpListenerStream};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, transport::Server};

const TOKEN: &str = "reader.private-test-secret";

#[derive(Clone, Default)]
struct Peer {
    retries: Arc<AtomicUsize>,
}

fn authenticated<T>(request: &Request<T>) {
    assert_eq!(
        request.metadata().get("authorization").unwrap(),
        format!("Bearer {TOKEN}").as_str()
    );
}

#[tonic::async_trait]
impl w::market_read_server::MarketRead for Peer {
    async fn books(&self, _: Request<w::BooksRequest>) -> Result<Response<w::BooksPage>, Status> {
        Err(Status::unimplemented(
            "replication does not fetch books separately",
        ))
    }

    type ReadStream = tokio_stream::Iter<std::vec::IntoIter<Result<w::Page, Status>>>;
    async fn read(
        &self,
        request: Request<w::ReadRequest>,
    ) -> Result<Response<Self::ReadStream>, Status> {
        authenticated(&request);
        assert!(request.metadata().contains_key("grpc-timeout"));
        let query = request.into_inner();
        assert!(query.selection.is_none());
        assert_eq!(query.mode, w::ReadMode::Snapshot as i32);
        assert_eq!(query.limit, 500);
        let count = match query.page_token.as_str() {
            "empty" => 0,
            "extra" => 2,
            "denied" => return Err(Status::permission_denied("denied")),
            "retry" if self.retries.fetch_add(1, Ordering::SeqCst) < 2 => {
                return Err(Status::resource_exhausted("busy"));
            }
            _ => 1,
        };
        Ok(Response::new(tokio_stream::iter(
            (0..count)
                .map(|_| {
                    Ok(w::Page {
                        cursor: "checkpoint".into(),
                        next_page_token: "next-page".into(),
                        ..Default::default()
                    })
                })
                .collect::<Vec<_>>(),
        )))
    }

    type WatchStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<w::Batch, Status>> + Send>>;
    async fn watch(
        &self,
        request: Request<w::WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        authenticated(&request);
        assert!(request.get_ref().selection.is_none());
        assert!(!request.metadata().contains_key("grpc-timeout"));
        Ok(Response::new(Box::pin(
            tokio_stream::once(Ok(w::Batch {
                cursor: request.into_inner().after_cursor,
                changes: vec![],
            }))
            .chain(tokio_stream::pending()),
        )))
    }
}

#[tokio::test]
async fn snapshot_and_watch_keep_authentication_and_continuations_inside_the_client() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = HubClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        TOKEN.into(),
    )
    .unwrap();
    let peer = Peer::default();
    let retries = peer.retries.clone();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(peer))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                cancel.cancelled_owned(),
            )
            .await
            .unwrap();
    });

    let page = client.snapshot_page("retry".into()).await.unwrap();
    assert_eq!(page.cursor, "checkpoint");
    assert_eq!(page.next_page_token, "next-page");
    assert_eq!(retries.load(Ordering::SeqCst), 3);
    for (token, code) in [
        ("empty", tonic::Code::DataLoss),
        ("extra", tonic::Code::DataLoss),
        ("denied", tonic::Code::PermissionDenied),
    ] {
        assert_eq!(
            client.snapshot_page(token.into()).await.unwrap_err().code(),
            code
        );
    }
    let mut stream = client.watch(page.cursor).await.unwrap();
    assert_eq!(
        stream.message().await.unwrap().unwrap().cursor,
        "checkpoint"
    );
    drop(stream);
    stop.cancel();
    server.await.unwrap();
}

#[tokio::test]
async fn source_binding_survives_token_rotation_and_never_contains_the_secret() {
    let client = HubClient::new("http://127.0.0.1:50051".into(), TOKEN.into()).unwrap();
    let rotated = HubClient::new(
        "http://127.0.0.1:50051".into(),
        "reader.rotated-private-secret".into(),
    )
    .unwrap();
    assert_eq!(client.source(), "http://127.0.0.1:50051|reader");
    assert_eq!(client.source(), rotated.source());
    assert!(client.auth.is_sensitive());
    for endpoint in [
        "https://example.com",
        "http://user:secret@example.com",
        "http://example.com/path",
        "http://example.com/?token=secret",
    ] {
        let error = HubClient::new(endpoint.into(), TOKEN.into()).err().unwrap();
        assert!(!error.to_string().contains("secret"));
    }
    for token in [
        "missing_separator",
        "bad id.private-secret",
        "reader.",
        "reader.private\nsecret",
    ] {
        let error = HubClient::new("http://127.0.0.1:50051".into(), token.into())
            .err()
            .unwrap();
        assert!(!error.to_string().contains(token));
    }
}
