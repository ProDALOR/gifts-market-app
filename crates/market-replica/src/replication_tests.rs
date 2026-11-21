use crate::ReplicaRuntime;
use crate::SyncState;
use crate::test_support::*;
use market_hub_protocol::wire as w;
use std::{sync::Arc, time::Duration};
use tonic::Status;
use w::entity::Entity as E;

#[tokio::test]
async fn draining_cancelled_storage_work_waits_for_returned_builder_cleanup() {
    use super::{blocking, drain};
    use tokio::sync::oneshot;
    use tokio_util::task::TaskTracker;

    struct HeldBuilder {
        _builder: crate::SnapshotBuilder,
        dropping: Option<oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl Drop for HeldBuilder {
        fn drop(&mut self) {
            let _ = self.dropping.take().unwrap().send(());
            let _ = self.release.recv();
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut writer = crate::ReplicaWriter::open(directory.path(), "cancelled-work").unwrap();
    let jobs = TaskTracker::new();
    let (started, running) = oneshot::channel();
    let (finish, finished) = std::sync::mpsc::channel();
    let (dropping, dropped) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let builder = writer.begin_snapshot().unwrap();
    let mut work = Box::pin(blocking(&jobs, move || {
        let _ = started.send(());
        let _ = finished.recv();
        Ok(HeldBuilder {
            _builder: builder,
            dropping: Some(dropping),
            release: released,
        })
    }));
    tokio::select! {
        _ = running => {},
        _ = &mut work => panic!("storage job must wait for cancellation"),
    }
    drop(work);
    finish.send(()).unwrap();
    dropped.await.unwrap();
    let mut drained = Box::pin(drain(&jobs));
    let premature = tokio::time::timeout(Duration::from_millis(25), &mut drained)
        .await
        .is_ok();
    // Always release the blocking destructor before asserting, including when
    // the regression occurs, so a failed assertion cannot hang runtime exit.
    release.send(()).unwrap();
    drained.await;
    assert!(
        !premature,
        "draining must include abandoned result destruction"
    );
    assert!(!writer.reader().status().ready);
    jobs.reopen();
    assert!(writer.begin_snapshot().is_ok());
    assert_eq!(blocking(&jobs, || Ok(42)).await.unwrap(), 42);
    drain(&jobs).await;
}

#[tokio::test]
async fn storage_panic_releases_tracking_and_preserves_the_published_replica() {
    use super::{blocking, drain};
    use tokio_util::task::TaskTracker;
    let directory = tempfile::tempdir().unwrap();
    let mut writer = crate::ReplicaWriter::open(directory.path(), "panicked-work").unwrap();
    let mut builder = writer.begin_snapshot().unwrap();
    builder.append(&catalog()).unwrap();
    writer
        .install_snapshot(builder.finish("saved").unwrap())
        .unwrap();
    let builder = writer.begin_snapshot().unwrap();
    let jobs = TaskTracker::new();
    let error = blocking::<()>(&jobs, move || {
        let _builder = builder;
        panic!("test storage worker panic");
    })
    .await
    .unwrap_err();
    drain(&jobs).await;
    assert_eq!(error.code(), tonic::Code::Unavailable);
    assert_eq!(writer.cursor().unwrap(), "saved");
    assert!(writer.reader().status().ready);
    assert!(writer.begin_snapshot().is_ok());
}

#[tokio::test]
async fn prefetch_cancellation_and_watch_failure_release_staging_before_recovery() {
    use tokio_stream::{StreamExt, wrappers::TcpListenerStream};
    use tonic::{Request, Response, transport::Server};
    type Stream<T> = std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<T, Status>> + Send>>;
    #[derive(Clone)]
    struct Peer {
        started: tokio::sync::mpsc::UnboundedSender<()>,
        watches: tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Sender<()>>,
        released: Arc<tokio::sync::Notify>,
    }
    #[tonic::async_trait]
    impl w::market_read_server::MarketRead for Peer {
        async fn books(
            &self,
            _: Request<w::BooksRequest>,
        ) -> Result<Response<w::BooksPage>, Status> {
            Err(Status::unimplemented("unused"))
        }
        type ReadStream = Stream<w::Page>;
        async fn read(
            &self,
            request: Request<w::ReadRequest>,
        ) -> Result<Response<Self::ReadStream>, Status> {
            let first = request.get_ref().page_token.is_empty();
            let peer = self.clone();
            Ok(Response::new(Box::pin(futures_util::stream::once(
                async move {
                    if first {
                        Ok(w::Page {
                            cursor: "baseline".into(),
                            entities: catalog()
                                .into_iter()
                                .map(|change| match change.change.unwrap() {
                                    w::change::Change::Upsert(entity) => entity,
                                    _ => unreachable!(),
                                })
                                .collect(),
                            next_page_token: "later".into(),
                        })
                    } else {
                        let _ = peer.started.send(());
                        peer.released.notified().await;
                        Ok(w::Page {
                            cursor: "baseline".into(),
                            entities: vec![entity(E::Order(order("listing", "9", None)))],
                            ..Default::default()
                        })
                    }
                },
            ))))
        }
        type WatchStream = Stream<w::Batch>;
        async fn watch(
            &self,
            _: Request<w::WatchRequest>,
        ) -> Result<Response<Self::WatchStream>, Status> {
            let (fail, failed) = tokio::sync::oneshot::channel();
            let _ = self.watches.send(fail);
            Ok(Response::new(Box::pin(
                tokio_stream::once(Ok(w::Batch {
                    cursor: "updated".into(),
                    changes: vec![upsert(entity(E::Order(order("listing", "1", None))))],
                }))
                .chain(futures_util::stream::once(async move {
                    if failed.await.is_err() {
                        std::future::pending::<()>().await;
                    }
                    Err(Status::unavailable(
                        "test Watch disconnected during prefetch",
                    ))
                })),
            )))
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = market_hub_client::HubClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        "test.private-test-token".into(),
    )
    .unwrap();
    let (started, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let (watches, mut watch_requests) = tokio::sync::mpsc::unbounded_channel();
    let released = Arc::new(tokio::sync::Notify::new());
    let server_stop = tokio_util::sync::CancellationToken::new();
    let server = tokio::spawn(
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(Peer {
                started,
                watches,
                released: released.clone(),
            }))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                server_stop.clone().cancelled_owned(),
            ),
    );
    for cancelled in [true, false] {
        let market = ReplicaRuntime::open(directory.path(), "prefetch-test")
            .await
            .unwrap();
        let reader = market.reader;
        let writer = market.replication.writer.clone();
        let stop = tokio_util::sync::CancellationToken::new();
        let worker = tokio::spawn(market.replication.run(hub.clone(), stop.clone()));
        tokio::time::timeout(Duration::from_secs(3), requests.recv())
            .await
            .unwrap()
            .unwrap();
        let fail_watch = watch_requests.recv().await.unwrap();
        if !cancelled {
            fail_watch.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(3), requests.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(
                !reader.status().ready,
                "a failed build must remain unpublished"
            );
            released.notify_one();
            tokio::time::timeout(Duration::from_secs(3), async {
                while reader.cursor().unwrap_or_default() != "updated" {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(3), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(reader.status().state, SyncState::Stale);
        assert_eq!(reader.status().ready, !cancelled);
        if cancelled {
            assert_eq!(writer.lock().unwrap().cursor().unwrap(), "");
            // No detached page/replay result may retain the old build lease.
            assert!(writer.lock().unwrap().begin_snapshot().is_ok());
        } else {
            assert_eq!(
                order_entity(&reader.read(read(1, false)).unwrap().page.entities[0])
                    .unwrap()
                    .price,
                "1"
            );
        }
    }
    server_stop.cancel();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn snapshot_paging_spools_changes_before_older_snapshot_rows_and_resumes_saved_state() {
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response, transport::Server};
    #[derive(Clone)]
    struct Peer {
        reads: Arc<std::sync::atomic::AtomicUsize>,
        watches: Arc<std::sync::atomic::AtomicUsize>,
        reset: Arc<std::sync::atomic::AtomicBool>,
        page_retried: Arc<std::sync::atomic::AtomicBool>,
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
            if r.get_ref().page_token == "later"
                && !self
                    .page_retried
                    .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(Status::unavailable("transient continuation failure"));
            }
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
        page_retried: Arc::default(),
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
        4,
        "retry only the failed page; restart must resume without a new snapshot"
    );
    assert_eq!(peer.watches.load(std::sync::atomic::Ordering::SeqCst), 4);
    stopped.cancel();
    server.await.unwrap();
}
