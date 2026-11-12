//! The gateway talks only to the copied protocol, with no market state or query engine.
#[path = "../../../tests/support/mod.rs"]
mod fixtures;
use hub_protocol::wire as w;
use market_app::client::CoreClient;
use market_app::{ApiState, query::BookQuery};
use market_view as m;
use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio_stream::{Stream, wrappers::TcpListenerStream};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status, transport::Server};
#[derive(Clone, Default)]
struct Peer(Arc<AtomicUsize>);
fn meta() -> Option<w::ReadMeta> {
    Some(w::ReadMeta {
        cursor: Some(w::Cursor {
            generation: "authoritative".into(),
            revision: 9007199254740993,
        }),
        catalog_revision: "catalog".into(),
        served_at: "2026-11-12T10:00:00Z".into(),
        as_of: "2026-11-12T10:00:00Z".into(),
    })
}
fn authorized<T>(r: &Request<T>) {
    assert!(
        r.metadata().get("authorization").unwrap()
            == "Bearer gateway.private-test-reader-signature"
    );
}
#[tonic::async_trait]
impl w::market_read_server::MarketRead for Peer {
    async fn get_status(&self, r: Request<w::Empty>) -> Result<Response<w::ReadStatus>, Status> {
        authorized(&r);
        Ok(Response::new(w::ReadStatus {
            ready: true,
            records: 17,
            feeds: 1,
            ready_feeds: 1,
            meta: meta(),
            ..Default::default()
        }))
    }
    async fn list_catalog(
        &self,
        r: Request<w::CatalogQuery>,
    ) -> Result<Response<w::CatalogResult>, Status> {
        authorized(&r);
        assert_eq!(r.get_ref().kind.as_deref(), Some("attribute"));
        Ok(Response::new(w::CatalogResult {
            data: vec![w::CatalogEntry {
                entry: Some(w::catalog_entry::Entry::Attribute(w::AttributeDefinition {
                    id: "model".into(),
                    name: "Model".into(),
                    value_type: w::AttributeType::Enum as i32,
                    ..Default::default()
                })),
            }],
            page: None,
            meta: meta(),
        }))
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
        Err(Status::invalid_argument("bad order filters"))
    }
    async fn list_stored_orders(
        &self,
        _: Request<w::OrdersQuery>,
    ) -> Result<Response<w::OrdersResult>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn get_order_book(
        &self,
        r: Request<w::BookQuery>,
    ) -> Result<Response<w::BookResult>, Status> {
        authorized(&r);
        self.0.fetch_add(1, Ordering::SeqCst);
        let q = r.into_inner();
        assert!(q.include_items && q.include_feeds);
        assert_eq!(
            q.attributes.unwrap().attributes.get("model").unwrap().value,
            Some(w::attribute_value::Value::EnumId("gold".into()))
        );
        let orders = fixtures::records()
            .into_iter()
            .filter_map(|r| match r {
                m::Record::Order(v) if v.side() == m::OrderSide::Ask => Some(w::Order::from(v)),
                _ => None,
            })
            .take(1)
            .collect();
        Ok(Response::new(w::BookResult {
            item_type_id: q.item_type_id,
            currency_id: q.currency_id,
            asks: Some(w::BookSidePage {
                data: orders,
                page: Some(w::Page {
                    next_cursor: Some("core-owned-cursor".into()),
                }),
            }),
            bids: Some(w::BookSidePage::default()),
            meta: meta(),
            included: None,
        }))
    }
    async fn list_trades(
        &self,
        _: Request<w::TradesQuery>,
    ) -> Result<Response<w::TradesResult>, Status> {
        let data = fixtures::sources()
            .into_iter()
            .filter_map(|e| match e {
                m::SourceEvent::Trade(t) => Some(t.into()),
                _ => None,
            })
            .collect();
        Ok(Response::new(w::TradesResult {
            data,
            page: Some(w::Page { next_cursor: None }),
            meta: meta(),
        }))
    }
    type WatchOrderBookStream = Pin<Box<dyn Stream<Item = Result<w::BookResult, Status>> + Send>>;
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
#[actix_web::test]
async fn rest_queries_forward_to_core_and_preserve_exact_values_dates_and_cursors() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = Peer::default();
    let service = peer.clone();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(service))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                cancel.cancelled_owned(),
            )
            .await
            .unwrap();
    });
    let node = CoreClient::new(
        format!("http://{address}"),
        "gateway.private-test-reader-signature".into(),
    )
    .unwrap();
    let state = ApiState::new(node, 8, 16);
    let app = actix_web::test::init_service(
        actix_web::App::new()
            .app_data(actix_web::web::Data::new(state.clone()))
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
    assert_eq!(value["data"]["records"], 17);
    assert_eq!(value["meta"]["cursor"]["revision"], "9007199254740993");
    let q = BookQuery {
        item_type_id: "test-item".parse().unwrap(),
        currency_id: "TON".parse().unwrap(),
        feed_id: None,
        item_id: None,
        attributes: Some("{\"model\":\"gold\"}".into()),
        price_min: None,
        price_max: None,
        limit: None,
        include_items: true,
        include_feeds: true,
    };
    let value = serde_json::to_value(state.orderbook(&q).await.unwrap()).unwrap();
    assert_eq!(value["data"]["asks"]["data"][0]["price"]["amount"], "12.50");
    assert_eq!(
        value["data"]["asks"]["page"]["next_cursor"],
        "core-owned-cursor"
    );
    assert!(value["data"]["asks"]["data"][0]["observed_at"].is_string());
    assert!(
        value["data"]["asks"]["data"][0]
            .get("observation")
            .is_none()
    );
    let mut bad = q.clone();
    bad.attributes = Some("{\"model\":true}".into());
    assert!(state.orderbook(&bad).await.is_err());
    bad.attributes = Some("{\"unknown\":\"value\"}".into());
    assert!(state.orderbook(&bad).await.is_err());
    assert_eq!(peer.0.load(Ordering::SeqCst), 1);
    let value: serde_json::Value = actix_web::test::call_and_read_body_json(
        &app,
        actix_web::test::TestRequest::get()
            .uri("/trades")
            .to_request(),
    )
    .await;
    assert_eq!(value["data"][0]["quantity"], "0.25");
    assert!(value["data"][0]["executed_at"].is_string());
    let response = actix_web::test::call_service(
        &app,
        actix_web::test::TestRequest::get()
            .uri("/orders")
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 400);
    let response = actix_web::test::call_service(
        &app,
        actix_web::test::TestRequest::post()
            .uri("/orders")
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 405);
    stop.cancel();
    server.await.unwrap();
}
