use crate::ReplicaRuntime;
use crate::SyncState;
use crate::test_support::*;
use market_hub_protocol::wire as w;
use std::{sync::Arc, time::Duration};
use tonic::Status;
use w::entity::Entity as E;
#[tokio::test]
async fn snapshot_paging_spools_changes_before_older_snapshot_rows_and_resumes_saved_state() {
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response, transport::Server};
    #[derive(Clone)]
    struct Peer {
        reads: Arc<std::sync::atomic::AtomicUsize>,
        watches: Arc<std::sync::atomic::AtomicUsize>,
        reset: Arc<std::sync::atomic::AtomicBool>,
    }
    #[tonic::async_trait]
    impl w::market_read_server::MarketRead for Peer {
        async fn books(
            &self,
            _: Request<w::BooksRequest>,
        ) -> Result<Response<w::BooksPage>, Status> {
            Err(Status::unimplemented("replication never needs Books"))
        }
        type ReadStream = tokio_stream::Once<Result<w::Page, Status>>;
        async fn read(
            &self,
            r: Request<w::ReadRequest>,
        ) -> Result<Response<Self::ReadStream>, Status> {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert!(r.get_ref().selection.is_none());
            if self.reset.load(std::sync::atomic::Ordering::SeqCst) {
                let mut entities = catalog()
                    .into_iter()
                    .map(|change| match change.change.unwrap() {
                        w::change::Change::Upsert(e) => e,
                        _ => unreachable!(),
                    })
                    .collect::<Vec<_>>();
                entities.push(entity(E::Order(order("new", "5", None))));
                return Ok(Response::new(tokio_stream::once(Ok(w::Page {
                    entities,
                    cursor: "new0".into(),
                    ..Default::default()
                }))));
            }
            let page = if r.get_ref().page_token.is_empty() {
                w::Page {
                    entities: catalog()
                        .into_iter()
                        .map(|c| match c.change.unwrap() {
                            w::change::Change::Upsert(e) => e,
                            _ => unreachable!(),
                        })
                        .collect(),
                    next_page_token: "later".into(),
                    cursor: "c0".into(),
                }
            } else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                w::Page {
                    entities: vec![entity(E::Order(order("late-row", "9", None)))],
                    cursor: "c0".into(),
                    ..Default::default()
                }
            };
            Ok(Response::new(tokio_stream::once(Ok(page))))
        }
        type WatchStream =
            std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<w::Batch, Status>> + Send>>;
        async fn watch(
            &self,
            r: Request<w::WatchRequest>,
        ) -> Result<Response<Self::WatchStream>, Status> {
            use tokio_stream::StreamExt;
            self.watches
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let cursor = r.into_inner().after_cursor;
            if self.reset.load(std::sync::atomic::Ordering::SeqCst) && cursor != "new0" {
                return Err(Status::failed_precondition("old generation"));
            }
            let frames = if cursor == "new0" {
                vec![Ok(w::Batch {
                    cursor,
                    changes: vec![],
                })]
            } else if cursor == "c0" {
                vec![
                    Ok(w::Batch {
                        cursor: "c0".into(),
                        changes: vec![],
                    }),
                    Ok(w::Batch {
                        cursor: "c1".into(),
                        changes: vec![
                            upsert(entity(E::Order(order("late-row", "1", None)))),
                            upsert(entity(E::Order(order("executed", "2", Some(1))))),
                        ],
                    }),
                ]
            } else {
                assert_eq!(cursor, "c1");
                vec![Ok(w::Batch {
                    cursor,
                    changes: vec![],
                })]
            };
            Ok(Response::new(Box::pin(
                tokio_stream::iter(frames).chain(tokio_stream::pending()),
            )))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = market_hub_client::HubClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        "test.private-test-token".into(),
    )
    .unwrap();
    let peer = Peer {
        reads: Arc::default(),
        watches: Arc::default(),
        reset: Arc::default(),
    };
    let server = peer.clone();
    let stopped = tokio_util::sync::CancellationToken::new();
    let cancel = stopped.clone();
    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(server))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                cancel.cancelled_owned(),
            )
            .await
            .unwrap();
    });
    for phase in 0..3 {
        let restored = phase > 0;
        if phase == 2 {
            peer.reset.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let market = ReplicaRuntime::open(dir.path(), "test-source")
            .await
            .unwrap();
        let replica = market.reader;
        if restored {
            assert!(replica.status().ready);
        }
        let cancel = tokio_util::sync::CancellationToken::new();
        let worker_stop = cancel.clone();
        let hub = hub.clone();
        let worker = tokio::spawn(market.replication.run(hub, worker_stop));
        tokio::time::timeout(Duration::from_secs(5), async {
            while replica.status().state != SyncState::Live
                || replica
                    .read(read(1, false))
                    .map(|v| v.page.cursor != if phase == 2 { "new0" } else { "c1" })
                    .unwrap_or(true)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let current = replica.read(read(1, false)).unwrap();
        assert_eq!(
            order_entity(&current.page.entities[0]).unwrap().price,
            if phase == 2 { "5" } else { "1" }
        );
        if phase == 2 {
            assert!(
                replica
                    .read(read(0, true))
                    .unwrap()
                    .page
                    .entities
                    .is_empty(),
                "replacement snapshots remove old history"
            );
        } else {
            assert_eq!(
                ids(&replica.read(read(0, true)).unwrap().page),
                ["executed"]
            );
        }
        cancel.cancel();
        worker.await.unwrap().unwrap();
        drop(replica);
    }
    assert_eq!(
        peer.reads.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "restart must resume without a new snapshot"
    );
    assert_eq!(peer.watches.load(std::sync::atomic::Ordering::SeqCst), 4);
    stopped.cancel();
    server.await.unwrap();
}
