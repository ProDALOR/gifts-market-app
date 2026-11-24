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
    streaming: Arc<AtomicUsize>,
    closed: Arc<AtomicUsize>,
    connections:
        Arc<std::sync::Mutex<std::collections::BTreeMap<&'static str, std::net::SocketAddr>>>,
    retries: Arc<AtomicUsize>,
    attempts: Arc<std::sync::Mutex<std::collections::BTreeMap<String, usize>>>,
}

fn authenticated<T>(request: &Request<T>) {
    assert_eq!(
        request.metadata().get("authorization").unwrap(),
        format!("Bearer {TOKEN}").as_str()
    );
}

#[tonic::async_trait]
impl w::market_read_server::MarketRead for Peer {
    type ReadSnapshotPartStream = Self::ReadStream;
    async fn open_snapshot(
        &self,
        request: Request<w::Empty>,
    ) -> Result<Response<w::SnapshotSession>, Status> {
        authenticated(&request);
        if self.streaming.load(Ordering::SeqCst) == 4 {
            return Err(Status::permission_denied("snapshot denied"));
        }
        if self.streaming.load(Ordering::SeqCst) < 5 {
            return Err(Status::unimplemented("coordinated snapshots unavailable"));
        }
        self.connections
            .lock()
            .unwrap()
            .insert("live", request.remote_addr().unwrap());
        Ok(Response::new(w::SnapshotSession {
            token: "session".into(),
            cursor: "checkpoint".into(),
        }))
    }
    async fn read_snapshot_part(
        &self,
        request: Request<w::SnapshotPartRequest>,
    ) -> Result<Response<Self::ReadSnapshotPartStream>, Status> {
        authenticated(&request);
        assert!(!request.metadata().contains_key("grpc-timeout"));
        let history = request.get_ref().part == w::SnapshotPart::History as i32;
        self.connections.lock().unwrap().insert(
            if history { "history" } else { "current" },
            request.remote_addr().unwrap(),
        );
        assert_eq!(request.get_ref().token, "session");
        let mode = self.streaming.load(Ordering::SeqCst);
        if mode == 8 && history {
            return Err(Status::unavailable("history setup failed"));
        }
        let entity = if history {
            w::entity::Entity::Order(w::Order {
                mode: w::OrderMode::Executed as i32,
                ..Default::default()
            })
        } else {
            w::entity::Entity::Currency(w::Currency::default())
        };
        let page = w::Page {
            entities: if mode == 9 && history {
                vec![w::Entity {
                    entity: Some(w::entity::Entity::Currency(w::Currency::default())),
                }]
            } else {
                vec![w::Entity {
                    entity: Some(entity),
                }]
            },
            cursor: if mode == 6 && history {
                "wrong-checkpoint"
            } else {
                "checkpoint"
            }
            .into(),
            next_page_token: if mode == 7 && history { "more" } else { "" }.into(),
        };
        let frames = if mode == 10 && history {
            vec![Ok(page.clone()), Ok(page)]
        } else {
            vec![Ok(page)]
        };
        Ok(Response::new(tokio_stream::iter(frames)))
    }
    async fn close_snapshot(
        &self,
        request: Request<w::SnapshotToken>,
    ) -> Result<Response<w::Empty>, Status> {
        authenticated(&request);
        assert_eq!(request.get_ref().token, "session");
        self.closed.fetch_add(1, Ordering::SeqCst);
        Ok(Response::new(w::Empty {}))
    }

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
        let attempt = {
            let mut attempts = self.attempts.lock().unwrap();
            let count = attempts.entry(query.page_token.clone()).or_default();
            *count += 1;
            *count
        };
        match query.page_token.as_str() {
            "unavailable" if attempt == 1 => {
                return Err(Status::unavailable("retry this continuation"));
            }
            "deadline" if attempt == 1 => {
                return Err(Status::deadline_exceeded("retry this continuation"));
            }
            "exhausted" => return Err(Status::unavailable("still unavailable")),
            "expired" => return Err(Status::failed_precondition("expired snapshot")),
            "stream-unavailable" if attempt == 1 => {
                return Ok(Response::new(tokio_stream::iter(vec![Err(
                    Status::unavailable("stream failed"),
                )])));
            }
            "after-page" if attempt == 1 => {
                return Ok(Response::new(tokio_stream::iter(vec![
                    Ok(w::Page {
                        cursor: "incomplete".into(),
                        ..Default::default()
                    }),
                    Err(Status::unavailable("stream ended incorrectly")),
                ])));
            }
            _ => {}
        }
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
                        next_page_token: if query.page_token == "next-page" {
                            ""
                        } else {
                            "next-page"
                        }
                        .into(),
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
    let attempts = peer.attempts.clone();
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

    let mut snapshot = client.snapshot().await.unwrap();
    assert_eq!(snapshot.transport(), "pages");
    assert!(
        !snapshot
            .next()
            .await
            .unwrap()
            .unwrap()
            .next_page_token
            .is_empty()
    );
    assert!(
        snapshot
            .next()
            .await
            .unwrap()
            .unwrap()
            .next_page_token
            .is_empty()
    );
    assert!(snapshot.next().await.unwrap().is_none());
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
    for token in [
        "unavailable",
        "deadline",
        "stream-unavailable",
        "after-page",
    ] {
        let result = client.snapshot_page(token.into()).await.unwrap();
        assert_eq!(
            result.cursor, "checkpoint",
            "A partial delivery cannot escape a retry"
        );
        assert_eq!(
            attempts.lock().unwrap()[token],
            2,
            "Retries preserve the exact continuation"
        );
    }
    assert_eq!(
        client
            .snapshot_page("exhausted".into())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unavailable
    );
    assert_eq!(attempts.lock().unwrap()["exhausted"], 6);
    assert_eq!(
        client
            .snapshot_page("expired".into())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    assert_eq!(attempts.lock().unwrap()["expired"], 1);
    assert_eq!(attempts.lock().unwrap()["denied"], 1);
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

#[tokio::test]
async fn snapshot_setup_does_not_fall_back_after_authorization_failure() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = HubClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        TOKEN.into(),
    )
    .unwrap();
    let peer = Peer::default();
    let mode = peer.streaming.clone();
    let attempts = peer.attempts.clone();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let server = tokio::spawn(
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(peer))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                cancel.cancelled_owned(),
            ),
    );
    mode.store(4, Ordering::SeqCst);
    assert_eq!(
        client.snapshot().await.err().unwrap().code(),
        tonic::Code::PermissionDenied
    );
    assert!(
        attempts.lock().unwrap().is_empty(),
        "stream failures cannot fall back to a different snapshot"
    );
    stop.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn coordinated_parts_use_independent_connections_validate_frames_and_release_sessions() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = HubClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        TOKEN.into(),
    )
    .unwrap();
    let peer = Peer::default();
    let mode = peer.streaming.clone();
    let closed = peer.closed.clone();
    let connections = peer.connections.clone();
    let stop = CancellationToken::new();
    let server = tokio::spawn(
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(peer))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                stop.clone().cancelled_owned(),
            ),
    );
    mode.store(5, Ordering::SeqCst);
    let mut snapshot = client.snapshot().await.unwrap();
    assert_eq!(snapshot.transport(), "split_stream");
    let mut rows = 0;
    while let Some(page) = snapshot.next().await.unwrap() {
        rows += page.entities.len();
    }
    assert_eq!(rows, 2);
    {
        let ports = connections
            .lock()
            .unwrap()
            .values()
            .map(|address| address.port())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            ports.len(),
            3,
            "Cloning a Channel is not connection isolation"
        );
    }
    drop(snapshot);
    for value in [6, 7, 9, 10] {
        mode.store(value, Ordering::SeqCst);
        let mut snapshot = client.snapshot().await.unwrap();
        loop {
            match snapshot.next().await {
                Ok(Some(_)) => {}
                Ok(None) => panic!("invalid snapshot accepted: mode {value}"),
                Err(error) => {
                    assert_eq!(error.code(), tonic::Code::DataLoss);
                    break;
                }
            }
        }
    }
    mode.store(8, Ordering::SeqCst);
    assert_eq!(
        client.snapshot().await.err().unwrap().code(),
        tonic::Code::Unavailable
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while closed.load(Ordering::SeqCst) < 5 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stop.cancel();
    server.await.unwrap().unwrap();
}
