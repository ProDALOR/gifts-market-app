use market_reader::sync::{Reader, Update};
use market_view::protocol::{catalog::CatalogSnapshot, wire as w};
use std::{pin::Pin, sync::Arc};
use tokio_stream::{Stream, StreamExt, wrappers::TcpListenerStream};
use tokio_util::sync::CancellationToken;
use tonic::{
    Request, Response, Status,
    transport::{Channel, Server},
};
#[derive(Clone)]
struct Source(Arc<Vec<w::SyncFrame>>, std::time::Duration);
#[tonic::async_trait]
impl w::market_replication_server::MarketReplication for Source {
    type SyncStream = Pin<Box<dyn Stream<Item = Result<w::SyncFrame, Status>> + Send>>;
    async fn sync(&self, _: Request<w::SyncRequest>) -> Result<Response<Self::SyncStream>, Status> {
        Ok(Response::new(Box::pin(
            tokio_stream::iter(self.0.as_ref().clone().into_iter().map(Ok)).throttle(self.1),
        )))
    }
}
fn frames() -> Vec<w::SyncFrame> {
    let catalog = CatalogSnapshot::new("g".into(), Arc::new(market_view::Catalog::default()));
    [w::FrameKind::SnapshotStart, w::FrameKind::SnapshotComplete]
        .into_iter()
        .map(|kind| w::SyncFrame {
            cursor: Some(w::Cursor {
                generation: "g".into(),
                revision: 0,
            }),
            kind: kind as i32,
            catalog_revision: catalog.revision.clone(),
            complete: kind == w::FrameKind::SnapshotComplete,
            ..Default::default()
        })
        .collect()
}
async fn reader(
    frames: Vec<w::SyncFrame>,
) -> (Reader, CancellationToken, tokio::task::JoinHandle<()>) {
    reader_with_limits(frames, Default::default(), std::time::Duration::ZERO).await
}
async fn reader_with_limits(
    frames: Vec<w::SyncFrame>,
    limits: market_reader::sync::Limits,
    delay: std::time::Duration,
) -> (Reader, CancellationToken, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let task = tokio::spawn(async move {
        Server::builder()
            .add_service(w::market_replication_server::MarketReplicationServer::new(
                Source(Arc::new(frames), delay),
            ))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                cancel.cancelled_owned(),
            )
            .await
            .unwrap();
    });
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    (
        Reader::connect(
            channel,
            &"Bearer test-reader".parse().unwrap(),
            None,
            w::ChangeProjection::Live,
            limits,
        )
        .await
        .unwrap(),
        stop,
        task,
    )
}
#[tokio::test]
async fn complete_snapshot_is_visible_only_after_its_marker() {
    let (mut reader, stop, task) = reader(frames()).await;
    assert!(
        matches!(reader.message().await.unwrap(),Some(Update::Snapshot(snapshot)) if snapshot.cursor.revision==0 && snapshot.records.is_empty())
    );
    assert!(reader.message().await.unwrap().is_none());
    drop(reader);
    stop.cancel();
    task.await.unwrap();
}
#[tokio::test]
async fn truncated_and_inconsistent_delta_chunks_never_become_updates() {
    for inconsistent in [false, true] {
        let mut input = frames();
        let revision = input[0].catalog_revision.clone();
        input.push(w::SyncFrame {
            cursor: Some(w::Cursor {
                generation: "g".into(),
                revision: 1,
            }),
            kind: w::FrameKind::Delta as i32,
            catalog_revision: revision.clone(),
            ..Default::default()
        });
        if inconsistent {
            input.push(w::SyncFrame {
                cursor: Some(w::Cursor {
                    generation: "g".into(),
                    revision: 2,
                }),
                kind: w::FrameKind::Delta as i32,
                catalog_revision: revision,
                complete: true,
                ..Default::default()
            });
        }
        let (mut reader, stop, task) = reader(input).await;
        assert!(matches!(
            reader.message().await.unwrap(),
            Some(Update::Snapshot(_))
        ));
        assert_eq!(
            reader.message().await.err().unwrap().code(),
            tonic::Code::DataLoss
        );
        drop(reader);
        stop.cancel();
        task.await.unwrap();
    }
}
#[tokio::test]
async fn snapshot_identity_counts_and_completion_are_checked() {
    for issue in 0..4 {
        let mut input = frames();
        match issue {
            0 => {
                input[1].catalog_revision = "another-revision".into();
            }
            1 => input[0].total_records = 1,
            2 => input[1].complete = false,
            _ => {
                input.pop();
            }
        }
        let (mut reader, stop, task) = reader(input).await;
        assert_eq!(
            reader.message().await.err().unwrap().code(),
            tonic::Code::DataLoss
        );
        drop(reader);
        stop.cancel();
        task.await.unwrap();
    }
}

#[tokio::test]
async fn active_chunk_progress_renews_contact_but_has_a_transaction_deadline() {
    use std::time::Duration;
    let entries: Vec<_> = (0..30)
        .map(|n| {
            market_view::CatalogEntry::Venue(market_view::VenueDefinition {
                id: format!("v{n}").parse().unwrap(),
                name: format!("Venue {n}"),
            })
        })
        .collect();
    let catalog = CatalogSnapshot::new(
        "g".into(),
        Arc::new(market_view::Catalog::from_entries(entries.clone()).unwrap()),
    );
    for hard_timeout in [false, true] {
        let mut input = frames();
        input[0].catalog_revision = catalog.revision.clone();
        input[1].catalog_revision = catalog.revision.clone();
        let complete = input.pop().unwrap();
        for entry in &entries {
            input.push(w::SyncFrame {
                cursor: input[0].cursor.clone(),
                kind: w::FrameKind::SnapshotData as i32,
                catalog_revision: catalog.revision.clone(),
                catalog: vec![entry.clone().into()],
                ..Default::default()
            });
        }
        input.push(complete);
        let limits = market_reader::sync::Limits {
            contact_timeout: Duration::from_millis(250),
            transaction_timeout: if hard_timeout {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(5)
            },
            ..Default::default()
        };
        let (mut reader, stop, task) =
            reader_with_limits(input, limits, Duration::from_millis(20)).await;
        if hard_timeout {
            assert_eq!(
                reader.message().await.err().unwrap().code(),
                tonic::Code::DeadlineExceeded
            );
        } else {
            assert!(matches!(
                reader.message().await.unwrap(),
                Some(Update::Snapshot(_))
            ));
        }
        drop(reader);
        stop.cancel();
        task.await.unwrap();
    }
}

#[tokio::test]
async fn catalog_revision_is_an_opaque_server_identity() {
    let mut input = frames();
    for frame in &mut input {
        frame.catalog_revision = "opaque-server-revision".into();
    }
    let (mut reader, stop, task) = reader(input).await;
    let Some(Update::Snapshot(snapshot)) = reader.message().await.unwrap() else {
        panic!("snapshot expected")
    };
    assert_eq!(snapshot.catalog.revision, "opaque-server-revision");
    drop(reader);
    stop.cancel();
    task.await.unwrap();
}
