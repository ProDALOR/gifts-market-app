//! The API is exercised against a protocol peer, never an imported hub implementation.
#[path = "../../../tests/support/mod.rs"]
mod fixtures;
use market_app::{
    ApiState,
    replication::{ReplicaConfig, ReplicaHandle},
};
use market_reader::registration::NodeCredentials;
use market_view::{
    self as m,
    protocol::{catalog::CatalogSnapshot, wire as w},
};
use prost::Message;
use std::{pin::Pin, sync::Arc, time::Duration};
use tokio_stream::{Stream, StreamExt, wrappers::TcpListenerStream};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status, transport::Server};
#[derive(Clone)]
struct Peer(Arc<Vec<w::SyncFrame>>);
#[tonic::async_trait]
impl w::market_registration_server::MarketRegistration for Peer {
    async fn register_node(
        &self,
        r: Request<w::RegisterNodeRequest>,
    ) -> Result<Response<w::NodeRegistration>, Status> {
        let r = r.into_inner();
        assert_eq!(r.role, w::NodeRole::Reader as i32);
        Ok(Response::new(w::NodeRegistration {
            id: r.id,
            role: r.role,
            duplicate: true,
        }))
    }
}
#[tonic::async_trait]
impl w::market_replication_server::MarketReplication for Peer {
    type SyncStream = Pin<Box<dyn Stream<Item = Result<w::SyncFrame, Status>> + Send>>;
    async fn sync(&self, r: Request<w::SyncRequest>) -> Result<Response<Self::SyncStream>, Status> {
        assert_eq!(r.into_inner().projection, w::ChangeProjection::Live as i32);
        Ok(Response::new(Box::pin(
            tokio_stream::iter(self.0.as_ref().clone().into_iter().map(Ok))
                .chain(tokio_stream::pending()),
        )))
    }
}
fn frames() -> Vec<w::SyncFrame> {
    let catalog = CatalogSnapshot::new(
        "contract-peer".into(),
        Arc::new(fixtures::catalog("source")),
    );
    let mut records = fixtures::records();
    let now = m::MarketTimestamp::now();
    for r in &mut records {
        match r {
            m::Record::Item(v) => v.observed_at = now,
            m::Record::Order(v) => v.observed_at = now,
            _ => {}
        }
    }
    records.push(m::Record::Feed(m::FeedStatus {
        feed_id: "source".parse().unwrap(),
        venue_id: catalog
            .catalog
            .feed(&"source".parse().unwrap())
            .unwrap()
            .venue_id
            .clone(),
        state: m::FeedState::Ready,
        last_sync_at: Some(now),
    }));
    records.sort_by_key(m::Record::key);
    let records: Vec<w::Record> = records.into_iter().map(Into::into).collect();
    let cursor = w::Cursor {
        generation: catalog.generation.clone(),
        revision: 3,
    };
    let base = w::SyncFrame {
        cursor: Some(cursor.clone()),
        catalog_revision: catalog.revision.clone(),
        ..Default::default()
    };
    vec![
        w::SyncFrame {
            kind: w::FrameKind::SnapshotStart as i32,
            total_records: records.len() as u64,
            record_bytes: records.iter().map(|r| r.encoded_len() as u64).sum(),
            ..base.clone()
        },
        w::SyncFrame {
            kind: w::FrameKind::SnapshotData as i32,
            catalog: catalog.entries.iter().cloned().map(Into::into).collect(),
            ..base.clone()
        },
        w::SyncFrame {
            kind: w::FrameKind::SnapshotData as i32,
            records,
            ..base.clone()
        },
        w::SyncFrame {
            kind: w::FrameKind::SnapshotComplete as i32,
            complete: true,
            ..base.clone()
        },
        w::SyncFrame {
            kind: w::FrameKind::Checkpoint as i32,
            complete: true,
            ..base
        },
    ]
}
#[actix_web::test]
async fn grpc_snapshot_drives_read_only_rest_and_exact_public_values() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn")
        .with_test_writer()
        .try_init();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let peer = Peer(Arc::new(frames()));
    let service = peer.clone();
    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(
                w::market_registration_server::MarketRegistrationServer::new(service.clone()),
            )
            .add_service(w::market_replication_server::MarketReplicationServer::new(
                service,
            ))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                cancel.cancelled_owned(),
            )
            .await
            .unwrap();
    });
    let replica = ReplicaHandle::new();
    let config = ReplicaConfig::new(
        format!("http://{address}"),
        NodeCredentials {
            id: "api-contract-test".into(),
            role: w::NodeRole::Reader,
            token: "private-test-reader-credential".into(),
        },
    );
    let handle = replica.clone();
    let cancel = stop.clone();
    let follower = tokio::spawn(async move {
        handle.run(config, cancel).await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if replica.snapshot().connected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let app = actix_web::test::init_service(
        actix_web::App::new()
            .app_data(actix_web::web::Data::new(ApiState::new(
                replica.clone(),
                Duration::from_secs(60),
                8,
                5000,
            )))
            .configure(market_app::configure),
    )
    .await;
    let value: serde_json::Value = actix_web::test::call_and_read_body_json(
        &app,
        actix_web::test::TestRequest::get()
            .uri("/readyz")
            .to_request(),
    )
    .await;
    assert_eq!(value["data"]["ready"], true);
    assert_eq!(value["meta"]["cursor"]["revision"], "3");
    let value: serde_json::Value = actix_web::test::call_and_read_body_json(
        &app,
        actix_web::test::TestRequest::get()
            .uri("/orders?item_type_id=test-item&currency_id=TON&side=ask&limit=10")
            .to_request(),
    )
    .await;
    let orders = value["data"].as_array().unwrap();
    assert!(!orders.is_empty());
    assert!(orders.iter().all(|r| r["observed_at"].is_string()
        && r["price"]["amount"].is_string()
        && r["quantity"].is_string()));
    for method in [
        actix_web::http::Method::POST,
        actix_web::http::Method::PUT,
        actix_web::http::Method::DELETE,
    ] {
        let response = actix_web::test::call_service(
            &app,
            actix_web::test::TestRequest::default()
                .method(method)
                .uri("/orders")
                .to_request(),
        )
        .await;
        assert!(response.status().is_client_error());
    }
    stop.cancel();
    follower.await.unwrap();
    server.await.unwrap();
}
