use actix_web::{App, HttpServer, test, web};
use futures_util::{SinkExt, StreamExt};
use market_app::{client::CoreClient, configure, gateway::Gateway, wire as w};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_stream::wrappers::{TcpListenerStream, WatchStream};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status, transport::Server};
use utoipa::OpenApi;
const TOKEN: &str = "public-app.private-test-service-token";
#[derive(Clone)]
struct Peer {
    page: tokio::sync::watch::Sender<w::Page>,
    reads: Arc<AtomicUsize>,
    watches: Arc<AtomicUsize>,
    _directory: Arc<tempfile::TempDir>,
    disconnect: CancellationToken,
}
#[tonic::async_trait]
impl w::market_read_server::MarketRead for Peer {
    async fn books(&self, r: Request<w::BooksRequest>) -> Result<Response<w::BooksPage>, Status> {
        assert_eq!(
            r.metadata().get("authorization").unwrap(),
            format!("Bearer {TOKEN}").as_str()
        );
        Ok(Response::new(w::BooksPage {
            books: vec![book()],
            next_after: None,
        }))
    }
    type ReadStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<w::Page, Status>> + Send>>;
    async fn read(&self, r: Request<w::ReadRequest>) -> Result<Response<Self::ReadStream>, Status> {
        assert_eq!(
            r.metadata().get("authorization").unwrap(),
            format!("Bearer {TOKEN}").as_str()
        );
        let query = r.into_inner();
        assert!(query.selection.is_none(), "public reads must stay local");
        assert_eq!(query.mode, w::ReadMode::Snapshot as i32);
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(Response::new(Box::pin(tokio_stream::once(Ok(self
            .page
            .borrow()
            .clone())))))
    }
    type WatchStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<w::Batch, Status>> + Send>>;
    async fn watch(
        &self,
        r: Request<w::WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        assert_eq!(
            r.metadata().get("authorization").unwrap(),
            format!("Bearer {TOKEN}").as_str()
        );
        assert!(r.get_ref().selection.is_none());
        self.watches.fetch_add(1, Ordering::SeqCst);
        Ok(Response::new(Box::pin(
            WatchStream::new(self.page.subscribe())
                .map(|page| {
                    Ok(w::Batch {
                        cursor: page.cursor,
                        changes: page
                            .entities
                            .into_iter()
                            .map(|entity| w::Change {
                                change: Some(w::change::Change::Upsert(entity)),
                            })
                            .collect(),
                    })
                })
                .take_until(self.disconnect.clone().cancelled_owned()),
        )))
    }
}
fn book() -> w::BookKey {
    w::BookKey {
        venue_id: "test".into(),
        item_type_id: "tg.gift.123".into(),
        currency_id: "GRAM".into(),
    }
}
fn page(price: &str) -> w::Page {
    w::Page {
        entities: [
            w::Entity {
                entity: Some(w::entity::Entity::Venue(w::Venue {
                    id: "test".into(),
                    name: "Test".into(),
                })),
            },
            w::Entity {
                entity: Some(w::entity::Entity::Currency(w::Currency {
                    id: "GRAM".into(),
                    name: "Gram".into(),
                    decimals: 9,
                })),
            },
            w::Entity {
                entity: Some(w::entity::Entity::ItemType(w::ItemType {
                    id: "tg.gift.123".into(),
                    name: "Gift".into(),
                    quantity_step: "1".into(),
                    ..Default::default()
                })),
            },
            w::Entity {
                entity: Some(w::entity::Entity::Order(w::Order {
                    id: "order".into(),
                    book: Some(book()),
                    mode: w::OrderMode::Ask as i32,
                    price: price.into(),
                    quantity: "1".into(),
                    observed_at_ms: 1800000000123,
                    attributes: [(
                        "number".into(),
                        w::AttributePredicate {
                            predicate: Some(w::attribute_predicate::Predicate::Eq(
                                w::AttributeValue {
                                    value: Some(w::attribute_value::Value::Integer(
                                        9007199254740993,
                                    )),
                                },
                            )),
                        },
                    )]
                    .into(),
                    ..Default::default()
                })),
            },
        ]
        .into(),
        next_page_token: String::new(),
        cursor: format!("checkpoint.{price}"),
    }
}
async fn setup() -> (Gateway, Peer, CancellationToken) {
    let (sender, _) = tokio::sync::watch::channel(page("1.000000001"));
    let peer = Peer {
        page: sender,
        reads: Arc::default(),
        watches: Arc::default(),
        _directory: Arc::new(tempfile::tempdir().unwrap()),
        disconnect: CancellationToken::new(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let node = CoreClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        TOKEN.into(),
    )
    .unwrap();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let server = peer.clone();
    tokio::spawn(async move {
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(server))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                cancel.cancelled_owned(),
            )
            .await
            .unwrap()
    });
    let replica = market_replica::Replica::open(peer._directory.path(), "test-source").unwrap();
    let target = replica.clone();
    let cancel = stop.clone();
    tokio::spawn(async move {
        target
            .run(node.channel(), node.authorization(), cancel)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !replica.status().ready {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    (Gateway::new(replica, 8, 8), peer, stop)
}
fn query() -> serde_json::Value {
    serde_json::json!({"scope":{"item_type_id":"tg.gift.123","currency_id":"GRAM","modes":["ask"]},"limit":25})
}
#[actix_web::test]
async fn rest_is_read_only_preserves_exact_values_and_never_exposes_service_credentials() {
    let (state, _, stop) = setup().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/orders/query")
            .set_json(query())
            .to_request(),
    )
    .await;
    assert!(response.status().is_success());
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["data"][0]["price"], "1.000000001");
    assert_eq!(
        body["data"][0]["attributes"]["number"]["value"]["value"],
        "9007199254740993"
    );
    assert!(
        body["data"][0]["observed_at"]
            .as_str()
            .unwrap()
            .contains('T')
    );
    assert!(body["data"][0].get("venue_id").is_none());
    assert!(!body.to_string().contains(TOKEN));
    for route in [
        "/items",
        "/trades",
        "/status",
        "/market.MarketWrite/Publish",
    ] {
        assert_eq!(
            test::call_service(&app, test::TestRequest::get().uri(route).to_request())
                .await
                .status(),
            404
        );
    }
    assert_eq!(test::call_service(&app,test::TestRequest::post().uri("/orders/query").set_json(serde_json::json!({"scope":{"item_type_id":"tg.gift.123","currency_id":"GRAM","modes":["executed"]}})).to_request()).await.status(),400);
    let schema = test::call_service(
        &app,
        test::TestRequest::get().uri("/openapi.json").to_request(),
    )
    .await;
    let schema: serde_json::Value = test::read_body_json(schema).await;
    assert!(schema["components"]["schemas"]["Command"].is_object());
    assert!(schema["components"]["schemas"]["AttributeValue"].is_object());
    assert!(schema["paths"]["/orders/query"].is_object());
    stop.cancel();
}
#[actix_web::test]
async fn public_websocket_queries_share_local_pages_and_one_replication_stream() {
    let (state, peer, stop) = setup().await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .app_data(web::Data::new(Vec::<String>::new()))
            .configure(configure)
    })
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    let url = format!("ws://{address}/ws");
    let (mut a, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let (mut b, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    for socket in [&mut a, &mut b] {
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({"type":"subscribe","id":"offers","query":query()})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    }
    async fn receive(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        price: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                if let tokio_tungstenite::tungstenite::Message::Text(text) = frame {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == "page" && value["page"]["data"][0]["price"] == price {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
    }
    receive(&mut a, "1.000000001").await;
    receive(&mut b, "1.000000001").await;
    assert_eq!(peer.reads.load(Ordering::SeqCst), 1);
    assert_eq!(peer.watches.load(Ordering::SeqCst), 1);
    peer.page.send_replace(page("2.000000002"));
    receive(&mut a, "2.000000002").await;
    receive(&mut b, "2.000000002").await;
    assert_eq!(peer.reads.load(Ordering::SeqCst), 1);
    assert_eq!(peer.watches.load(Ordering::SeqCst), 1);
    peer.disconnect.cancel();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = a.next().await {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "page" && value["page"]["sync"]["state"] == "stale" {
                    assert_eq!(value["page"]["data"][0]["price"], "2.000000002");
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    a.close(None).await.unwrap();
    b.close(None).await.unwrap();
    handle.stop(false).await;
    stop.cancel();
}
#[actix_web::test]
async fn public_contract_contains_no_legacy_money_item_or_trade_models() {
    let value = ApiDoc::openapi();
    let json = value.to_json().unwrap();
    assert!(json.contains("BookKey"));
    assert!(!json.contains("Money"));
}
use market_app::ApiDoc;
