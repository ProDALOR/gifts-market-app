use crate::ReplicaRuntime;
use crate::SyncState;
use crate::test_support::*;
use market_hub_protocol::wire as w;
use std::{sync::Arc, time::Duration};
use tonic::Status;
use w::entity::Entity as E;

#[tokio::test]
async fn abandoned_returned_snapshot_results_release_resources_off_thread_and_drain() {
    use super::StorageValue;
    use crate::replication::storage_jobs::StorageJobs;
    struct Held {
        _builder: crate::SnapshotBuilder,
        entered: Option<tokio::sync::oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl Drop for Held {
        fn drop(&mut self) {
            let _ = self.entered.take().unwrap().send(());
            let _ = self.release.recv_timeout(Duration::from_secs(5));
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut writer = crate::ReplicaWriter::open(directory.path(), "cleanup").unwrap();
    let jobs = StorageJobs::new().unwrap();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let value = StorageValue::new(
        Held {
            _builder: writer.begin_snapshot().unwrap(),
            entered: Some(entered),
            release: blocked,
        },
        &jobs,
    );
    jobs.close();
    let owner_untracked = tokio::time::timeout(Duration::from_millis(25), jobs.wait())
        .await
        .is_ok();
    drop(value);
    started.await.unwrap();
    let exclusive = writer.begin_snapshot().is_err();
    let mut draining = Box::pin(jobs.drain());
    let premature = tokio::time::timeout(Duration::from_millis(25), &mut draining)
        .await
        .is_ok();
    let _ = release.send(());
    draining.await;
    assert!(
        !owner_untracked,
        "returned values remain tracked while awaiting prefetch"
    );
    assert!(
        exclusive,
        "cleanup must retain the build lease until resources are released"
    );
    assert!(!premature, "recovery must drain off-thread result cleanup");
    assert!(writer.begin_snapshot().is_ok());
}

#[tokio::test]
async fn live_grouping_preserves_bounds_pending_frames_and_terminal_errors() {
    use super::live_deliveries;
    use crate::limits::{MAX_APPLY_BATCHES, MAX_APPLY_BYTES};
    use prost::Message;

    let frame = |index| w::Batch {
        cursor: format!("c{index}"),
        changes: vec![],
    };
    let mut stream = futures_util::stream::iter((0..130).map(|index| Ok(frame(index))));
    let mut pending = None;
    let mut batches = Vec::with_capacity(MAX_APPLY_BATCHES);
    let allocation = batches.as_ptr();
    let mut count = 0;
    for expected in [64, 64, 2] {
        let deliveries = live_deliveries(&mut stream, &mut pending, batches)
            .await
            .unwrap();
        assert_eq!(deliveries.batches.len(), expected);
        for batch in &deliveries.batches {
            assert_eq!(batch.cursor, format!("c{count}"));
            count += 1;
        }
        assert_eq!(deliveries.disconnected.is_some(), count == 130);
        assert!(pending.is_none());
        batches = deliveries.batches;
        assert_eq!(batches.as_ptr(), allocation, "group storage must be reused");
        batches.clear();
    }
    assert_eq!(MAX_APPLY_BATCHES, 64);
    let mut large = order("large", "1", None);
    large.meta.insert("payload".into(), "x".repeat(600 * 1024));
    let mut stream = futures_util::stream::iter((0..3).map(|index| {
        Ok(w::Batch {
            cursor: format!("large{index}"),
            changes: vec![upsert(entity(E::Order(large.clone())))],
        })
    }));
    for index in 0..3 {
        let deliveries = live_deliveries(&mut stream, &mut pending, Vec::new())
            .await
            .unwrap();
        assert_eq!(deliveries.batches.len(), 1);
        assert_eq!(deliveries.batches[0].cursor, format!("large{index}"));
        assert!(deliveries.batches[0].encoded_len() <= MAX_APPLY_BYTES);
        assert_eq!(pending.is_some(), index < 2);
    }
    let mut stream =
        futures_util::stream::iter([Ok(frame(0)), Err(Status::out_of_range("expired"))]);
    let deliveries = live_deliveries(&mut stream, &mut pending, Vec::new())
        .await
        .unwrap();
    assert_eq!(deliveries.batches[0].cursor, "c0");
    assert_eq!(
        deliveries.disconnected.unwrap().code(),
        tonic::Code::OutOfRange
    );
}

#[tokio::test]
async fn live_grouping_does_not_wait_for_another_delivery() {
    use super::live_deliveries;
    use futures_util::FutureExt;
    use tokio_stream::StreamExt;
    let mut stream = tokio_stream::once(Ok(w::Batch {
        cursor: "ready".into(),
        changes: vec![],
    }))
    .chain(tokio_stream::pending());
    let mut pending = None;
    let deliveries = live_deliveries(&mut stream, &mut pending, Vec::new())
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(deliveries.batches.len(), 1);
    assert_eq!(deliveries.batches[0].cursor, "ready");
    assert!(deliveries.disconnected.is_none());
}

#[cfg(feature = "profile-allocations")]
#[tokio::test]
#[ignore = "isolated allocation benchmark; use --test-threads=1"]
async fn live_group_buffer_allocation_benchmark() {
    use super::live_deliveries;
    use crate::limits::MAX_APPLY_BATCHES;
    let frames: Vec<_> = (0..1024)
        .map(|index| {
            Ok(w::Batch {
                cursor: format!("c{index}"),
                changes: vec![],
            })
        })
        .collect();
    let mut stream = futures_util::stream::iter(frames);
    let mut pending = None;
    let mut batches = Vec::with_capacity(MAX_APPLY_BATCHES);
    let allocation = batches.as_ptr();
    let region = stats_alloc::Region::new(&stats_alloc::INSTRUMENTED_SYSTEM);
    for _ in 0..16 {
        let deliveries = live_deliveries(&mut stream, &mut pending, batches)
            .await
            .unwrap();
        assert_eq!(deliveries.batches.len(), MAX_APPLY_BATCHES);
        assert!(deliveries.disconnected.is_none());
        batches = deliveries.batches;
        assert_eq!(batches.as_ptr(), allocation);
        batches.clear();
    }
    println!(
        "grouping 1,024 decoded frames into 16 groups: {:?}",
        region.change()
    );
}

#[tokio::test]
async fn cancellation_replays_buffered_frames_after_the_last_admitted_group() {
    use super::live_deliveries;
    use crate::replication::storage_jobs::StorageJobs;
    use futures_util::StreamExt;

    for admitted in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let market = ReplicaRuntime::open(directory.path(), "buffered-cancellation")
            .await
            .unwrap();
        {
            let mut writer = market.replication.writer.lock().unwrap();
            let mut builder = writer.begin_snapshot().unwrap();
            builder.append(&catalog()).unwrap();
            writer
                .install_snapshot(builder.finish("baseline").unwrap())
                .unwrap();
        }
        let frames: Vec<_> = (0..3)
            .map(|index| {
                let mut row = order(&format!("o{index}"), "1", None);
                row.meta.insert("payload".into(), "x".repeat(600 * 1024));
                w::Batch {
                    cursor: format!("c{index}"),
                    changes: vec![upsert(entity(E::Order(row)))],
                }
            })
            .collect();
        let mut stream = futures_util::stream::iter(frames.iter().cloned().map(Ok))
            .chain(futures_util::stream::pending());
        let mut pending = None;
        let deliveries = live_deliveries(&mut stream, &mut pending, Vec::new())
            .await
            .unwrap();
        assert_eq!(deliveries.batches.len(), 1);
        assert_eq!(pending.as_ref().unwrap().cursor, "c1");
        let jobs = StorageJobs::new().unwrap();
        if admitted {
            let (started, running) = tokio::sync::oneshot::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            let mut work = Box::pin(market.replication.mutate(&jobs, move |writer| {
                let _ = started.send(());
                let _ = blocked.recv();
                writer.apply_deliveries(&deliveries.batches)
            }));
            tokio::select! {
                _ = running => {},
                _ = &mut work => panic!("admitted write must be blocked"),
            }
            drop(work);
            drop(pending);
            drop(stream);
            release.send(()).unwrap();
        } else {
            drop(deliveries);
            drop(pending);
            drop(stream);
        }
        jobs.drain().await;
        let first = usize::from(admitted);
        {
            let mut writer = market.replication.writer.lock().unwrap();
            writer.mark(SyncState::Stale);
            assert_eq!(
                writer.resume_cursor().unwrap(),
                if admitted { "c0" } else { "baseline" }
            );
        }
        // A new connection owns a new buffer. Its replay starts strictly after
        // the committed group, not after the frame read ahead by the old one.
        let replay = futures_util::stream::iter(frames.into_iter().skip(first).map(Ok)).chain(
            futures_util::stream::once(async {
                Err(Status::unavailable("closed after complete frames"))
            }),
        );
        tokio::pin!(replay);
        let mut pending = None;
        let mut batches = Vec::new();
        for index in first..3 {
            let mut deliveries = live_deliveries(&mut replay, &mut pending, batches)
                .await
                .unwrap();
            assert_eq!(deliveries.batches[0].cursor, format!("c{index}"));
            market
                .replication
                .writer
                .lock()
                .unwrap()
                .apply_deliveries(&deliveries.batches)
                .unwrap();
            assert_eq!(market.reader.cursor().unwrap(), format!("c{index}"));
            assert_eq!(deliveries.disconnected.is_some(), index == 2);
            deliveries.batches.clear();
            batches = deliveries.batches;
        }
        // These deliberately large transport fixtures exceed the separate
        // public page budget; inspect the replicated state directly.
        assert_eq!(
            market
                .reader
                .view()
                .unwrap()
                .state
                .range(None, Some(crate::types::EntityKind::Order))
                .count(),
            3
        );
    }
}

#[tokio::test]
async fn reconnect_after_failed_publication_repairs_history_and_replays_remaining_frames() {
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response, transport::Server};
    type Stream<T> = std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<T, Status>> + Send>>;
    #[derive(Clone)]
    struct Peer {
        cursors: tokio::sync::mpsc::UnboundedSender<String>,
        fail_watch_setup: Arc<std::sync::atomic::AtomicBool>,
    }
    #[tonic::async_trait]
    impl w::market_read_server::MarketRead for Peer {
        type ReadSnapshotPartStream = Self::ReadStream;
        async fn open_snapshot(
            &self,
            _: tonic::Request<w::Empty>,
        ) -> Result<tonic::Response<w::SnapshotSession>, tonic::Status> {
            Err(tonic::Status::unimplemented(
                "coordinated snapshots unavailable",
            ))
        }
        async fn read_snapshot_part(
            &self,
            _: tonic::Request<w::SnapshotPartRequest>,
        ) -> Result<tonic::Response<Self::ReadSnapshotPartStream>, tonic::Status> {
            Err(tonic::Status::unimplemented(
                "coordinated snapshots unavailable",
            ))
        }
        async fn close_snapshot(
            &self,
            _: tonic::Request<w::SnapshotToken>,
        ) -> Result<tonic::Response<w::Empty>, tonic::Status> {
            Ok(tonic::Response::new(w::Empty {}))
        }

        async fn books(
            &self,
            _: Request<w::BooksRequest>,
        ) -> Result<Response<w::BooksPage>, Status> {
            Err(Status::unimplemented("unused"))
        }
        type ReadStream = Stream<w::Page>;
        async fn read(
            &self,
            _: Request<w::ReadRequest>,
        ) -> Result<Response<Self::ReadStream>, Status> {
            panic!("ordinary publication recovery must not request a hub snapshot");
        }
        type WatchStream = Stream<w::Batch>;
        async fn watch(
            &self,
            request: Request<w::WatchRequest>,
        ) -> Result<Response<Self::WatchStream>, Status> {
            use futures_util::StreamExt;
            let cursor = request.into_inner().after_cursor;
            let first = match cursor.as_str() {
                "baseline" => 0,
                "c0" => 1,
                _ => panic!("unexpected resume cursor: {cursor}"),
            };
            let _ = self.cursors.send(cursor);
            if self
                .fail_watch_setup
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(Status::unavailable("test transient Watch setup failure"));
            }
            let frames = (first..3).map(|index| {
                let mut row = order(&format!("o{index}"), "1", (index == 0).then_some(42));
                row.meta.insert("payload".into(), "x".repeat(600 * 1024));
                Ok(w::Batch {
                    cursor: format!("c{index}"),
                    changes: vec![upsert(entity(E::Order(row)))],
                })
            });
            Ok(Response::new(Box::pin(
                futures_util::stream::iter(frames).chain(futures_util::stream::pending()),
            )))
        }
    }
    for snapshot_publication in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let market = ReplicaRuntime::open(directory.path(), "publication-reconnect")
            .await
            .unwrap();
        {
            let mut writer = market.replication.writer.lock().unwrap();
            let mut builder = writer.begin_snapshot().unwrap();
            builder.append(&catalog()).unwrap();
            writer.fail_publish_once = snapshot_publication;
            let result = writer.install_snapshot(builder.finish("baseline").unwrap());
            assert_eq!(result.is_err(), snapshot_publication);
            writer.mark(SyncState::Stale);
            if !snapshot_publication {
                writer.fail_publish_once = true;
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let hub = market_hub_client::HubClient::new(
            format!("http://{}", listener.local_addr().unwrap()),
            "test.private-test-token".into(),
        )
        .unwrap();
        let (cursors, mut resumes) = tokio::sync::mpsc::unbounded_channel();
        let server_stop = tokio_util::sync::CancellationToken::new();
        let server = tokio::spawn(
            Server::builder()
                .add_service(w::market_read_server::MarketReadServer::new(Peer {
                    cursors,
                    fail_watch_setup: Arc::new(std::sync::atomic::AtomicBool::new(
                        snapshot_publication,
                    )),
                }))
                .serve_with_incoming_shutdown(
                    TcpListenerStream::new(listener),
                    server_stop.clone().cancelled_owned(),
                ),
        );
        let stop = tokio_util::sync::CancellationToken::new();
        let reader = market.reader;
        let history = reader.read_revision(&read(0, true)).ok();
        let worker = tokio::spawn(market.replication.run(hub, stop.clone()));
        tokio::time::timeout(Duration::from_secs(5), async {
            while reader.cursor().unwrap_or_default() != "c2" {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(resumes.recv().await.unwrap(), "baseline");
        if snapshot_publication {
            assert_eq!(
                resumes.recv().await.unwrap(),
                "baseline",
                "a failed Watch setup must resume the prepared snapshot without a Read"
            );
        } else {
            assert_eq!(resumes.recv().await.unwrap(), "c0");
        }
        let view = reader.view().unwrap();
        assert_eq!(
            view.state
                .range(None, Some(crate::types::EntityKind::Order))
                .count(),
            2
        );
        let sale = view
            .history
            .execution(&crate::types::Position {
                executed_at_ms: 42,
                venue_id: "venue".into(),
                order_id: "o0".into(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(order_entity(&sale).unwrap().id, "o0");
        assert_ne!(history, Some(reader.read_revision(&read(0, true)).unwrap()));
        assert_eq!(reader.status().state, SyncState::Live);
        stop.cancel();
        worker.await.unwrap().unwrap();
        server_stop.cancel();
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn draining_cancelled_storage_work_waits_for_returned_builder_cleanup() {
    use crate::replication::storage_jobs::StorageJobs;
    use tokio::sync::oneshot;

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
    let jobs = StorageJobs::new().unwrap();
    let (started, running) = oneshot::channel();
    let (finish, finished) = std::sync::mpsc::channel();
    let (dropping, dropped) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let builder = writer.begin_snapshot().unwrap();
    let mut work = Box::pin(jobs.run(move || {
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
    let mut drained = Box::pin(jobs.drain());
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
    assert_eq!(jobs.run(|| Ok(42)).await.unwrap(), 42);
    jobs.drain().await;
}

#[tokio::test]
async fn cancellation_drains_an_admitted_prepared_publication_retry_without_rebuilding() {
    use crate::replication::storage_jobs::StorageJobs;
    let directory = tempfile::tempdir().unwrap();
    let market = ReplicaRuntime::open(directory.path(), "cancelled-publication")
        .await
        .unwrap();
    let prepared = {
        let mut writer = market.replication.writer.lock().unwrap();
        let mut builder = writer.begin_snapshot().unwrap();
        builder.append(&catalog()).unwrap();
        writer
            .install_snapshot(builder.finish("baseline").unwrap())
            .unwrap();
        writer.mark(SyncState::Stale);
        writer.fail_publish_once = true;
        assert!(
            writer
                .apply(w::Batch {
                    cursor: "next".into(),
                    changes: vec![upsert(entity(E::Order(order("listing", "1", None))))]
                })
                .is_err()
        );
        match &writer.publication {
            crate::PublicationState::Prepared { view, .. } => view.clone(),
            _ => unreachable!(),
        }
    };
    let jobs = StorageJobs::new().unwrap();
    let (started, running) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let mut work = Box::pin(market.replication.mutate(&jobs, move |writer| {
        let _ = started.send(());
        let _ = blocked.recv();
        writer.resume_cursor()
    }));
    tokio::select! { _ = running => {}, _ = &mut work => panic!("retry must be blocked") }
    drop(work);
    release.send(()).unwrap();
    jobs.drain().await;
    assert!(prepared.revisions.sequence == market.reader.view().unwrap().revisions.sequence);
    assert_eq!(market.reader.cursor().unwrap(), "next");
    assert_eq!(market.reader.status().state, SyncState::Stale);
    let mut writer = market.replication.writer.lock().unwrap();
    assert!(matches!(
        writer.publication,
        crate::PublicationState::Current
    ));
    writer.resume_cursor().unwrap();
    assert!(prepared.revisions.sequence == market.reader.view().unwrap().revisions.sequence);
}

#[tokio::test]
async fn storage_panic_releases_tracking_and_preserves_the_published_replica() {
    use crate::replication::storage_jobs::StorageJobs;
    let directory = tempfile::tempdir().unwrap();
    let mut writer = crate::ReplicaWriter::open(directory.path(), "panicked-work").unwrap();
    let mut builder = writer.begin_snapshot().unwrap();
    builder.append(&catalog()).unwrap();
    writer
        .install_snapshot(builder.finish("saved").unwrap())
        .unwrap();
    let builder = writer.begin_snapshot().unwrap();
    let jobs = StorageJobs::new().unwrap();
    let error = jobs
        .run::<()>(move || {
            let _builder = builder;
            panic!("test storage worker panic");
        })
        .await
        .unwrap_err();
    jobs.drain().await;
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
        streaming: Arc<std::sync::atomic::AtomicBool>,
        started: tokio::sync::mpsc::UnboundedSender<()>,
        watches: tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Sender<()>>,
        released: Arc<tokio::sync::Notify>,
    }
    #[tonic::async_trait]
    impl w::market_read_server::MarketRead for Peer {
        type ReadSnapshotPartStream = Self::ReadStream;
        async fn open_snapshot(
            &self,
            _: Request<w::Empty>,
        ) -> Result<Response<w::SnapshotSession>, Status> {
            if !self.streaming.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(Status::unimplemented("coordinated snapshots unavailable"));
            }
            Ok(Response::new(w::SnapshotSession {
                token: "session".into(),
                cursor: "baseline".into(),
            }))
        }
        async fn read_snapshot_part(
            &self,
            request: Request<w::SnapshotPartRequest>,
        ) -> Result<Response<Self::ReadSnapshotPartStream>, Status> {
            if request.get_ref().part == w::SnapshotPart::History as i32 {
                return Ok(Response::new(Box::pin(tokio_stream::once(Ok(w::Page {
                    cursor: "baseline".into(),
                    ..Default::default()
                })))));
            }
            let first = self
                .read(Request::new(w::ReadRequest::default()))
                .await?
                .into_inner();
            let later = self
                .read(Request::new(w::ReadRequest {
                    page_token: "later".into(),
                    ..Default::default()
                }))
                .await?
                .into_inner();
            Ok(Response::new(Box::pin(first.chain(later))))
        }
        async fn close_snapshot(
            &self,
            _: tonic::Request<w::SnapshotToken>,
        ) -> Result<tonic::Response<w::Empty>, tonic::Status> {
            Ok(tonic::Response::new(w::Empty {}))
        }

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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = market_hub_client::HubClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        "test.private-test-token".into(),
    )
    .unwrap();
    let (started, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let (watches, mut watch_requests) = tokio::sync::mpsc::unbounded_channel();
    let released = Arc::new(tokio::sync::Notify::new());
    let streaming = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_stop = tokio_util::sync::CancellationToken::new();
    let server = tokio::spawn(
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(Peer {
                streaming: streaming.clone(),
                started,
                watches,
                released: released.clone(),
            }))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                server_stop.clone().cancelled_owned(),
            ),
    );
    for use_stream in [false, true] {
        streaming.store(use_stream, std::sync::atomic::Ordering::SeqCst);
        let directory = tempfile::tempdir().unwrap();
        for cancelled in [true, false] {
            // A recovered session leaves an unused failure trigger behind.
            // Each case must control its own Watch, rather than a previous one.
            while watch_requests.try_recv().is_ok() {}
            while requests.try_recv().is_ok() {}
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
        type ReadSnapshotPartStream = Self::ReadStream;
        async fn open_snapshot(
            &self,
            _: tonic::Request<w::Empty>,
        ) -> Result<tonic::Response<w::SnapshotSession>, tonic::Status> {
            Err(tonic::Status::unimplemented(
                "coordinated snapshots unavailable",
            ))
        }
        async fn read_snapshot_part(
            &self,
            _: tonic::Request<w::SnapshotPartRequest>,
        ) -> Result<tonic::Response<Self::ReadSnapshotPartStream>, tonic::Status> {
            Err(tonic::Status::unimplemented(
                "coordinated snapshots unavailable",
            ))
        }
        async fn close_snapshot(
            &self,
            _: tonic::Request<w::SnapshotToken>,
        ) -> Result<tonic::Response<w::Empty>, tonic::Status> {
            Ok(tonic::Response::new(w::Empty {}))
        }

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
                tokio::time::sleep(Duration::from_millis(10)).await;
                let index = r.get_ref().page_token.strip_prefix("later").unwrap();
                let index: usize = if index.is_empty() {
                    1
                } else {
                    index.parse().unwrap()
                };
                w::Page {
                    entities: vec![entity(E::Order(order("late-row", "9", None)))],
                    cursor: "c0".into(),
                    next_page_token: if index < 18 {
                        format!("later{}", index + 1)
                    } else {
                        String::new()
                    },
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
        21,
        "retry only the failed page; restart must resume without a new snapshot"
    );
    assert_eq!(peer.watches.load(std::sync::atomic::Ordering::SeqCst), 4);
    stopped.cancel();
    server.await.unwrap();
}
