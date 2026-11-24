use super::*;
use market_hub_client::HubClient;
use std::sync::{Arc, Mutex};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, transport::Server};

fn page(rows: usize, payload: usize) -> w::Page {
    w::Page {
        cursor: "baseline".into(),
        entities: (0..rows)
            .map(|i| w::Entity {
                entity: Some(w::entity::Entity::Currency(w::Currency {
                    id: i.to_string(),
                    name: "x".repeat(payload),
                    ..Default::default()
                })),
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn single_pages_are_checked_before_staging_and_batches_preserve_entity_order() {
    assert!(SnapshotBatch::first(page(MAX_SNAPSHOT_ROWS + 1, 0)).is_err());
    assert!(SnapshotBatch::first(page(1, MAX_SNAPSHOT_BYTES)).is_err());
    let batch = SnapshotBatch::first(page(3, 0)).unwrap();
    let ids: Vec<_> = batch
        .into_changes()
        .into_iter()
        .map(|change| {
            let Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::Currency(currency)),
            })) = change.change
            else {
                panic!("currency")
            };
            currency.id
        })
        .collect();
    assert_eq!(ids, ["0", "1", "2"]);
}

#[test]
fn snapshot_diagnostics_separate_catalog_feeds_active_orders_and_executions() {
    let mut source = page(1, 0);
    source.entities.push(w::Entity {
        entity: Some(w::entity::Entity::Feed(w::Feed::default())),
    });
    for mode in [w::OrderMode::Ask, w::OrderMode::Bid, w::OrderMode::Executed] {
        source.entities.push(w::Entity {
            entity: Some(w::entity::Entity::Order(w::Order {
                mode: mode as i32,
                ..Default::default()
            })),
        });
    }
    let bytes = source.encoded_len() as u64;
    let batch = SnapshotBatch::first(source).unwrap();
    assert_eq!(batch.rows, 5);
    assert_eq!(
        (
            batch.stats.catalog,
            batch.stats.feeds,
            batch.stats.asks,
            batch.stats.bids,
            batch.stats.executions
        ),
        (1, 1, 1, 1, 1)
    );
    assert_eq!(batch.stats.bytes, bytes);
}

#[derive(Clone)]
struct Peer {
    pages: Arc<Vec<w::Page>>,
    reads: Arc<Mutex<Vec<usize>>>,
    gate: Option<Arc<Gate>>,
}
struct Gate {
    once: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
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

    async fn books(&self, _: Request<w::BooksRequest>) -> Result<Response<w::BooksPage>, Status> {
        Err(Status::unimplemented("unused"))
    }
    type ReadStream = tokio_stream::Once<Result<w::Page, Status>>;
    async fn read(
        &self,
        request: Request<w::ReadRequest>,
    ) -> Result<Response<Self::ReadStream>, Status> {
        let index: usize = request.get_ref().page_token.parse().unwrap_or(0);
        self.reads.lock().unwrap().push(index);
        if index == 1
            && let Some(gate) = &self.gate
            && !gate.once.swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        let mut page = self.pages[index].clone();
        if index + 1 < self.pages.len() {
            page.next_page_token = (index + 1).to_string();
        }
        Ok(Response::new(tokio_stream::once(Ok(page))))
    }
    type WatchStream = tokio_stream::Once<Result<w::Batch, Status>>;
    async fn watch(
        &self,
        _: Request<w::WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        Err(Status::unimplemented("unused"))
    }
}

#[tokio::test]
async fn prefetch_bounds_pages_rows_bytes_and_carries_overflow_without_rereading() {
    for (pages, expected) in [
        (vec![page(0, 0); 19], vec![8, 8, 3]),
        (
            vec![page(2_500, 0), page(2_000, 0), page(1_000, 0)],
            vec![1, 2],
        ),
        (vec![page(1, MAX_SNAPSHOT_BYTES / 5); 6], vec![4, 2]),
    ] {
        let count = pages.len();
        let reads = Arc::new(Mutex::new(Vec::new()));
        let peer = Peer {
            pages: Arc::new(pages),
            reads: reads.clone(),
            gate: None,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let hub = HubClient::new(
            format!("http://{}", listener.local_addr().unwrap()),
            "test.private-test-token".into(),
        )
        .unwrap();
        let stop = CancellationToken::new();
        let server_stop = stop.clone();
        let server = tokio::spawn(
            Server::builder()
                .add_service(w::market_read_server::MarketReadServer::new(peer))
                .serve_with_incoming_shutdown(
                    TcpListenerStream::new(listener),
                    server_stop.cancelled_owned(),
                ),
        );
        let mut reader = SnapshotPages::new(hub.snapshot().await.unwrap(), "baseline".into());
        let mut batches = Vec::new();
        while let Some(batch) = reader.next_batch().await.unwrap() {
            assert!(batch.bytes <= MAX_SNAPSHOT_BYTES);
            assert!(batch.rows <= MAX_SNAPSHOT_ROWS);
            batches.push(batch.pages.len());
        }
        assert_eq!(batches, expected);
        assert_eq!(*reads.lock().unwrap(), (0..count).collect::<Vec<_>>());
        assert!(reader.next_batch().await.unwrap().is_none());
        stop.cancel();
        server.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn prefetched_checkpoint_changes_fail_before_a_batch_can_be_staged() {
    let peer = Peer {
        pages: Arc::new(vec![page(1, 0)]),
        reads: Default::default(),
        gate: None,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = HubClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        "test.private-test-token".into(),
    )
    .unwrap();
    let stop = CancellationToken::new();
    let server = tokio::spawn(
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(peer))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                stop.clone().cancelled_owned(),
            ),
    );
    let mut reader = SnapshotPages::new(hub.snapshot().await.unwrap(), "different".into());
    assert_eq!(
        reader.next_batch().await.err().unwrap().code(),
        tonic::Code::DataLoss
    );
    assert_eq!(
        reader.next_batch().await.err().unwrap().code(),
        tonic::Code::DataLoss,
        "An error must not consume the continuation and turn into EOF"
    );
    stop.cancel();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn cancelled_prefetch_retains_its_prefix_and_retries_the_same_continuation() {
    let gate = Arc::new(Gate {
        once: false.into(),
        entered: Default::default(),
        release: Default::default(),
    });
    let reads = Arc::new(Mutex::new(Vec::new()));
    let peer = Peer {
        pages: Arc::new(vec![page(1, 0); 3]),
        reads: reads.clone(),
        gate: Some(gate.clone()),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = HubClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        "test.private-test-token".into(),
    )
    .unwrap();
    let stop = CancellationToken::new();
    let server = tokio::spawn(
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(peer))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                stop.clone().cancelled_owned(),
            ),
    );
    let mut reader = SnapshotPages::new(hub.snapshot().await.unwrap(), "baseline".into());
    let mut fetch = Box::pin(reader.next_batch());
    tokio::select! {
        _ = gate.entered.notified() => {},
        _ = &mut fetch => panic!("second page should be pending"),
    }
    drop(fetch);
    gate.release.notify_one();
    let batch = reader.next_batch().await.unwrap().unwrap();
    assert_eq!(
        batch.pages.len(),
        3,
        "the completed prefix survives cancellation"
    );
    assert_eq!(*reads.lock().unwrap(), [0, 1, 1, 2]);
    assert!(reader.next_batch().await.unwrap().is_none());
    stop.cancel();
    server.await.unwrap().unwrap();
}
