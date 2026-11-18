use actix_web::{App, test, web};
use futures_util::{FutureExt, SinkExt, StreamExt};
use market_actors::{MarketDataHandle, MarketDataService};
#[cfg(feature = "miniapp")]
use market_api::miniapp::MiniAppApi;
use market_api::{ApiConfig, ApiServer, MarketApi, configure};
use market_hub_client::HubClient;
use market_hub_protocol::wire as w;
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
                    aliases: vec![w::ExternalId {
                        namespace: "tg".into(),
                        id: "123".into(),
                    }],
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
                    item_id: Some("9007199254740993".into()),
                    attributes: [(
                        "tg.number".into(),
                        w::AttributePredicate {
                            predicate: Some(w::attribute_predicate::Predicate::Eq(
                                w::AttributeValue {
                                    value: Some(w::attribute_value::Value::Integer(123)),
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
struct TestMarket {
    data: MarketDataHandle,
    valuation: market_valuation::ValuationHandle,
    stop: CancellationToken,
    jobs: Vec<tokio::task::JoinHandle<Result<(), Status>>>,
    _cancel_on_drop: tokio_util::sync::DropGuard,
}
impl TestMarket {
    async fn open(directory: &std::path::Path, hub: HubClient) -> Self {
        let replica = market_replica::ReplicaRuntime::open(directory, "test-source")
            .await
            .unwrap();
        let stop = CancellationToken::new();
        let data = MarketDataService::start_with_shutdown(replica.reader, 8, 8, stop.child_token())
            .await
            .unwrap();
        let valuation = market_valuation::Service::start_on(
            &data,
            "model_floor",
            market_valuation::FloorActor::model,
            Default::default(),
        )
        .await
        .unwrap();
        let collection = market_valuation::Service::start_on(
            &data,
            "collection_floor",
            market_valuation::FloorActor::collection,
            Default::default(),
        )
        .await
        .unwrap();
        let model_ratio = market_valuation::Service::start_on(
            &data,
            "model_multiplier",
            market_valuation::ModelMultiplierActor::new,
            Default::default(),
        )
        .await
        .unwrap();
        let backdrop_ratio = market_valuation::Service::start_on(
            &data,
            "backdrop_multiplier",
            market_valuation::BackdropMultiplierActor::new,
            Default::default(),
        )
        .await
        .unwrap();
        let registry = market_valuation::ValuationHandle::new([
            market_valuation::RegisteredStrategy::from(valuation.handle.clone()),
            market_valuation::RegisteredStrategy::from(collection.handle.clone()),
            market_valuation::RegisteredStrategy::from(model_ratio.handle.clone()),
            market_valuation::RegisteredStrategy::from(backdrop_ratio.handle.clone()),
        ])
        .unwrap();
        let collection_stop = stop.child_token();
        let valuation_stop = stop.child_token();
        let jobs = vec![
            tokio::spawn(data.task.run(stop.child_token())),
            tokio::spawn(async move {
                valuation
                    .task
                    .run(valuation_stop)
                    .await
                    .map_err(|error| Status::unavailable(error.to_string()))
            }),
            tokio::spawn(async move {
                collection
                    .task
                    .run(collection_stop)
                    .await
                    .map_err(|error| Status::unavailable(error.to_string()))
            }),
            tokio::spawn(
                model_ratio
                    .task
                    .run(stop.child_token())
                    .map(|r| r.map_err(|e| Status::unavailable(e.to_string()))),
            ),
            tokio::spawn(
                backdrop_ratio
                    .task
                    .run(stop.child_token())
                    .map(|r| r.map_err(|e| Status::unavailable(e.to_string()))),
            ),
            tokio::spawn(replica.replication.run(hub, stop.child_token())),
        ];
        Self {
            data: data.handle,
            valuation: registry,
            stop: stop.clone(),
            jobs,
            _cancel_on_drop: stop.drop_guard(),
        }
    }
    async fn shutdown(self) -> Result<(), Status> {
        self.stop.cancel();
        for job in self.jobs {
            job.await.unwrap()?;
        }
        Ok(())
    }
}
fn api_server(market: &TestMarket, web_dir: std::path::PathBuf) -> ApiServer {
    ApiServer::bind(
        ApiConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            workers: 2,
            web_dir: Some(web_dir),
            cors_origins: vec![],
            limits: Default::default(),
        },
        market.data.clone(),
        market.valuation.clone(),
    )
    .unwrap()
}
async fn setup() -> (MarketApi, TestMarket, Peer, CancellationToken) {
    let (sender, _) = tokio::sync::watch::channel(page("1.000000001"));
    let peer = Peer {
        page: sender,
        reads: Arc::default(),
        watches: Arc::default(),
        _directory: Arc::new(tempfile::tempdir().unwrap()),
        disconnect: CancellationToken::new(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let node = HubClient::new(
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
    let runtime = TestMarket::open(peer._directory.path(), node).await;
    let api = MarketApi::new(runtime.data.clone(), runtime.valuation.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        while api.ready().await.is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    (api, runtime, peer, stop)
}
fn query() -> serde_json::Value {
    serde_json::json!({"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"side":"ask","limit":25})
}
#[actix_web::test]
async fn rest_is_read_only_preserves_exact_values_and_never_exposes_service_credentials() {
    let (state, runtime, _, stop) = setup().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(configure),
    )
    .await;
    for (name, status) in [
        ("".to_owned(), 400),
        (" \t".to_owned(), 400),
        ("x".repeat(257), 400),
        ("💎".repeat(65), 400),
        ("x".repeat(256), 422),
        ("💎".repeat(64), 422),
    ] {
        let response = test::call_service(&app, test::TestRequest::post()
            .uri("/portfolio/value")
            .set_json(serde_json::json!({"strategy":"model_floor","positions":[{"model":{"kind":"name","collection":"Gift","model":name},"quantity":1}],"currency_id":"GRAM"}))
            .to_request()).await;
        assert_eq!(response.status(), status);
        if status == 400 {
            let body: serde_json::Value = test::read_body_json(response).await;
            assert_eq!(body["error"]["code"], "invalid_query");
            assert_eq!(
                body["error"]["message"],
                "collection and model names must contain 1..256 bytes"
            );
        }
    }
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
    assert_eq!(body["data"][0]["target"]["gift"]["number"], 123);
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
    assert_eq!(test::call_service(&app,test::TestRequest::post().uri("/orders/query").set_json(serde_json::json!({"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM","mode":"executed"}})).to_request()).await.status(),400);
    // REST and WebSocket requests use the same compiler, including gift-filter defaults and limits.
    for limit in [0, 201] {
        let mut invalid = query();
        invalid["limit"] = limit.into();
        assert_eq!(
            test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/orders/query")
                    .set_json(invalid)
                    .to_request()
            )
            .await
            .status(),
            400
        );
    }
    let defaults: serde_json::Value = test::read_body_json(test::call_service(&app,
        test::TestRequest::post().uri("/orders/query")
            .set_json(serde_json::json!({"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"}}))
            .to_request()).await).await;
    assert_eq!(defaults["data"][0]["mode"], "ask");
    let directory: serde_json::Value = test::read_body_json(
        test::call_service(&app, test::TestRequest::get().uri("/catalog").to_request()).await,
    )
    .await;
    assert_eq!(directory["books"][0]["currency_id"], "GRAM");
    assert_eq!(directory["collections"][0]["name"], "Gift");
    assert!(directory.get("sync").is_none());
    assert!(directory.get("cursor").is_none());
    assert_eq!(directory["books"][0]["collection_id"], "tg.gift.123");
    assert!(directory["books"][0].get("item_type_id").is_none());
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/collections/tg.gift.123")
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let collection: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(collection["collection"]["id"], "tg.gift.123");
    assert!(collection.get("sync").is_none());
    assert!(collection.get("cursor").is_none());
    let status: serde_json::Value = test::read_body_json(
        test::call_service(&app, test::TestRequest::get().uri("/sync").to_request()).await,
    )
    .await;
    assert_eq!(status["ready"], true);
    assert!(status["state"].is_string());
    assert!(status.get("updated_at").is_some());
    // Other response kinds retain their own freshness envelope.
    assert!(body["sync"]["ready"].as_bool().unwrap());
    for field in ["models", "backdrops", "patterns"] {
        assert!(collection[field].is_array());
    }
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/collections/missing")
                .to_request()
        )
        .await
        .status(),
        404
    );
    for route in ["/catalog/query", "/executions/query"] {
        assert_eq!(
            test::call_service(
                &app,
                test::TestRequest::post()
                    .uri(route)
                    .set_json(serde_json::json!({}))
                    .to_request()
            )
            .await
            .status(),
            404
        );
    }
    for invalid in [
        serde_json::json!({"scope":{"item_type_id":"tg.gift.123","currency_id":"GRAM"}}),
        serde_json::json!({"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM","attributes":{}}}),
        serde_json::json!({"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM","number":{"min":0}}}),
        serde_json::json!({"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM","price":{"min":"2","max":"1"}}}),
    ] {
        assert_eq!(
            test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/orders/query")
                    .set_json(invalid)
                    .to_request()
            )
            .await
            .status(),
            400
        );
    }

    let schema = test::call_service(
        &app,
        test::TestRequest::get().uri("/openapi.json").to_request(),
    )
    .await;
    let schema: serde_json::Value = test::read_body_json(schema).await;
    assert!(schema["components"]["schemas"]["Command"].is_object());
    assert!(schema["components"]["schemas"]["GiftTarget"].is_object());
    assert!(
        schema["components"]["schemas"]
            .get("AttributeValue")
            .is_none()
    );
    assert!(schema["paths"]["/orders/query"].is_object());
    for catalog in ["GiftCatalog", "GiftCollectionCatalog"] {
        let properties = &schema["components"]["schemas"][catalog]["properties"];
        assert!(properties.get("sync").is_none());
        assert!(properties.get("cursor").is_none());
    }
    for path in ["/catalog", "/collections/{id}", "/sync"] {
        assert!(schema["paths"][path].is_object());
    }
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
#[actix_web::test]
async fn public_websocket_queries_share_local_pages_and_one_replication_stream() {
    let (_state, runtime, peer, stop) = setup().await;
    let web_dir = peer._directory.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<html></html>").unwrap();
    let server = api_server(&runtime, web_dir);
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let api_task = tokio::spawn(server.run(api_stop.child_token()));
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
                        assert_eq!(
                            value["page"]["data"][0]["target"]["gift"]["id"],
                            "9007199254740993"
                        );
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
                if value["type"] == "sync" && value["sync"]["state"] == "stale" {
                    assert!(value.get("page").is_none());
                    assert!(value["revision"].is_string());
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    a.close(None).await.unwrap();
    b.close(None).await.unwrap();
    api_stop.cancel();
    api_task.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
#[actix_web::test]
async fn public_contract_contains_no_legacy_money_item_or_trade_models() {
    let value = ApiDoc::openapi();
    let json = value.to_json().unwrap();
    assert!(json.contains("GiftBook"));
    for name in [
        "BookKey",
        "AttributeValue",
        "Predicate",
        "Scope",
        "CatalogEntry",
    ] {
        assert!(
            !value
                .components
                .as_ref()
                .unwrap()
                .schemas
                .contains_key(name)
        );
    }
}
use market_api::ApiDoc;

#[cfg(not(feature = "miniapp"))]
#[actix_web::test]
async fn api_only_build_omits_miniapp_routes_schemas_and_external_browser_permissions() {
    let app = test::init_service(
        App::new()
            .wrap(market_api::security_headers())
            .configure(configure),
    )
    .await;
    for (method, path) in [
        (actix_web::http::Method::GET, "/telegram"),
        (actix_web::http::Method::GET, "/auth/me"),
        (actix_web::http::Method::POST, "/auth/telegram"),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::default()
                .method(method)
                .uri(path)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 404);
    }
    let response =
        test::call_service(&app, test::TestRequest::get().uri("/healthz").to_request()).await;
    assert_eq!(response.status(), 200);
    assert!(
        !response
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("telegram")
    );
    let schema: serde_json::Value =
        serde_json::from_str(&market_api::schema_json().unwrap()).unwrap();
    assert!(schema["paths"].get("/auth/telegram").is_none());
    assert!(schema["paths"].get("/auth/me").is_none());
    assert!(schema["paths"].get("/telegram").is_none());
    assert!(schema["components"]["schemas"].get("Session").is_none());
    assert!(
        schema["components"]["schemas"]
            .get("TelegramConfig")
            .is_none()
    );
}

#[cfg(feature = "miniapp")]
#[actix_web::test]
#[ignore = "requires the production React build and Playwright Chromium"]
async fn telegram_react_bridge_sign_in_launch_theme_back_button_and_reload() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use ed25519_dalek::{Signer, SigningKey};
    let (_state, runtime, _peer, stop) = setup().await;
    let signing = SigningKey::from_bytes(&[23; 32]);
    let auth =
        market_miniapp::auth::Auth::new(42, signing.verifying_key().to_bytes(), [61; 32]).unwrap();
    let telegram = MiniAppApi::new(
        Some(auth),
        Some("test_market_bot".into()),
        Some("https://markets.example".into()),
    )
    .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    assert!(root.join("web/dist/index.html").is_file());
    let server = api_server(&runtime, root.join("web/dist")).with_miniapp(telegram);
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let api_task = tokio::spawn(server.run(api_stop.child_token()));
    let selection = "collection=tg.gift.123&currency=GRAM&mode=ask&sort=price_desc&max=2";
    let payload = format!("v_{}", URL_SAFE_NO_PAD.encode(selection));
    let values: std::collections::BTreeMap<String, String> = [
        (
            "auth_date".into(),
            jiff::Timestamp::now().as_second().to_string(),
        ),
        ("query_id".into(), "test-query".into()),
        ("start_param".into(), payload),
        (
            "user".into(),
            r#"{"id":9007199254740991,"first_name":"Anna","username":"anna"}"#.into(),
        ),
    ]
    .into();
    let mut check = "42:WebAppData".to_owned();
    for (key, value) in &values {
        check.push_str(&format!("\n{key}={value}"));
    }
    let proof = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(values)
        .append_pair(
            "signature",
            &URL_SAFE_NO_PAD.encode(signing.sign(check.as_bytes()).to_bytes()),
        )
        .finish();
    let output = tokio::process::Command::new("node")
        .arg("scripts/telegram-smoke.mjs")
        .current_dir(root.join("web"))
        .env("MARKET_TELEGRAM_TEST_URL", format!("http://{address}"))
        .env("MARKET_TELEGRAM_TEST_PROOF", proof)
        .env(
            "MARKET_TELEGRAM_SCREENSHOTS",
            root.join("target/telegram-screenshots"),
        )
        .output();
    let result = tokio::time::timeout(Duration::from_secs(60), output)
        .await
        .unwrap()
        .unwrap();
    api_stop.cancel();
    api_task.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[cfg(not(feature = "miniapp"))]
#[actix_web::test]
#[ignore = "requires the API-only React build and Playwright Chromium"]
async fn api_only_browser_browses_without_telegram_requests_or_controls() {
    let (_state, runtime, _peer, stop) = setup().await;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let server = api_server(&runtime, root.join("web/dist"));
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let api_task = tokio::spawn(server.run(api_stop.child_token()));
    let result = tokio::time::timeout(
        Duration::from_secs(45),
        tokio::process::Command::new("node")
            .arg("scripts/api-only-smoke.mjs")
            .current_dir(root.join("web"))
            .env("MARKET_API_ONLY_TEST_URL", format!("http://{address}"))
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    api_stop.cancel();
    api_task.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

async fn http(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    headers: &str,
) -> (u16, String, serde_json::Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = vec![];
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    let response = String::from_utf8(response).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(body).unwrap_or_else(|_| serde_json::Value::String(body.into()))
    };
    (status, headers.into(), body)
}
#[actix_web::test]
async fn headless_server_normalizes_extractors_framework_errors_and_origin_policy() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let (_, runtime, _, stop) = setup().await;
    let config = ApiConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        workers: 1,
        cors_origins: vec!["https://EXAMPLE.com:443/".into()],
        limits: market_api::ApiLimits {
            json_bytes: 256,
            websocket: market_api::WebSocketLimits {
                connections: 1,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let server = ApiServer::bind(config, runtime.data.clone(), runtime.valuation.clone()).unwrap();
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    assert_eq!(http(address, "GET", "/healthz", "", "").await.0, 200);
    for (method, path, body, headers, status, code) in [
        ("GET", "/books?limit=bad", "", "", 404, "not_found"),
        ("GET", "/catalog/not_a_kind", "", "", 404, "not_found"),
        ("GET", "/collections/%20", "", "", 400, "invalid_query"),
        ("GET", "/media/models/%20", "", "", 400, "invalid_query"),
        (
            "GET",
            "/media/models/%20/animation",
            "",
            "",
            400,
            "invalid_query",
        ),
        ("GET", "/media/patterns/%20", "", "", 400, "invalid_query"),
        ("GET", "/missing", "", "", 404, "not_found"),
        ("GET", "/", "", "", 404, "not_found"),
        ("GET", "/assets/absent.js", "", "", 404, "not_found"),
        ("GET", "/orders/query", "", "", 405, "method_not_allowed"),
        (
            "POST",
            "/orders/query",
            "bad",
            "Content-Type: application/json\r\n",
            400,
            "invalid_query",
        ),
        (
            "POST",
            "/orders/query",
            "{}",
            "Content-Type: text/plain\r\n",
            415,
            "unsupported_media_type",
        ),
    ] {
        let (actual, headers, body) = http(address, method, path, body, headers).await;
        assert_eq!(actual, status, "{path}: {body}");
        assert!(
            headers
                .to_lowercase()
                .contains("content-type: application/json")
        );
        assert_eq!(body["error"]["code"], code, "{path}");
    }
    let (status, _, body) = http(
        address,
        "POST",
        "/orders/query",
        &format!("{{\"large\":\"{}\"}}", "x".repeat(512)),
        "Content-Type: application/json\r\n",
    )
    .await;
    assert_eq!(status, 413);
    assert_eq!(body["error"]["code"], "payload_too_large");
    let (status, headers, _) = http(
        address,
        "GET",
        "/healthz",
        "",
        "Origin: https://example.com\r\n",
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        headers
            .to_lowercase()
            .contains("access-control-allow-origin: https://example.com")
    );
    let (status, headers, _) = http(
        address,
        "GET",
        "/healthz",
        "",
        "Origin: https://denied.example\r\n",
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        !headers
            .to_lowercase()
            .contains("access-control-allow-origin")
    );
    let (status, headers, body) = http(
        address,
        "OPTIONS",
        "/orders/query",
        "",
        "Origin: https://example.com\r\nAccess-Control-Request-Method: DELETE\r\n",
    )
    .await;
    assert_eq!(status, 400);
    assert!(
        headers
            .to_lowercase()
            .contains("content-type: application/json")
    );
    assert!(body["error"].is_object());
    let mut denied = format!("ws://{address}/ws").into_client_request().unwrap();
    denied
        .headers_mut()
        .insert("Origin", "https://denied.example".parse().unwrap());
    assert!(
        matches!(tokio_tungstenite::connect_async(denied).await.unwrap_err(),tokio_tungstenite::tungstenite::Error::Http(response) if response.status()==403)
    );
    let mut request = format!("ws://{address}/ws").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Origin", "https://example.com".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let second = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap_err();
    assert!(
        matches!(second,tokio_tungstenite::tungstenite::Error::Http(response) if response.status()==429)
    );
    socket.close(None).await.unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
#[actix_web::test]
async fn oversized_subscription_pages_report_terminal_errors_and_release_the_id() {
    let (_, runtime, _, stop) = setup().await;
    let server = ApiServer::bind(
        ApiConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            workers: 1,
            limits: market_api::ApiLimits {
                websocket: market_api::WebSocketLimits {
                    page_bytes: 64,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        },
        runtime.data.clone(),
        runtime.valuation.clone(),
    )
    .unwrap();
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    for _ in 0..2 {
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({"type":"subscribe","id":"offers","query":query()})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let error = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                    socket.next().await
                {
                    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if frame["type"] == "error" {
                        break frame;
                    }
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(error["id"], "offers");
        assert_eq!(error["error"]["code"], "capacity_exceeded");
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("too large")
        );
    }
    // The server must be able to cancel and drain while the socket is still open.
    api_stop.cancel();
    tokio::time::timeout(Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

#[actix_web::test]
async fn live_rest_continuations_do_not_consume_retained_snapshot_capacity() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let directory = tempfile::tempdir().unwrap();
    let mut writer = market_replica::ReplicaWriter::open(directory.path(), "scroll-test").unwrap();
    let mut rows = page("1.000000001").entities;
    let mut second = rows
        .iter()
        .find_map(|entity| match &entity.entity {
            Some(w::entity::Entity::Order(order)) => Some(order.clone()),
            _ => None,
        })
        .unwrap();
    second.id = "second".into();
    second.price = "2".into();
    rows.push(w::Entity {
        entity: Some(w::entity::Entity::Order(second)),
    });
    let mut snapshot = writer.begin_snapshot().unwrap();
    snapshot
        .append(
            &rows
                .into_iter()
                .map(|entity| w::Change {
                    change: Some(w::change::Change::Upsert(entity)),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    writer
        .install_snapshot(snapshot.finish("scroll-cursor").unwrap())
        .unwrap();
    let service = MarketDataService::start(writer.reader(), 2, 2)
        .await
        .unwrap();
    let valuation = market_valuation::Service::start_on(
        &service,
        "model_floor",
        market_valuation::FloorActor::model,
        Default::default(),
    )
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(MarketApi::new(
                service.handle,
                market_valuation::ValuationHandle::new([valuation.handle.clone()]).unwrap(),
            )))
            .configure(configure),
    )
    .await;
    let token_json = |token: &str| {
        serde_json::from_slice::<serde_json::Value>(&URL_SAFE_NO_PAD.decode(token).unwrap())
            .unwrap()
    };
    let mut body = query();
    body["limit"] = 1.into();
    body["consistency"] = "live".into();
    let mut token = String::new();
    for _ in 0..160 {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&body)
                .to_request(),
        )
        .await;
        assert_eq!(
            response.status(),
            200,
            "live requests must not fill the 128-view snapshot budget"
        );
        let response: serde_json::Value = test::read_body_json(response).await;
        token = response["next_page_token"].as_str().unwrap().to_owned();
        assert!(token_json(&token)["snapshot"].is_null());
    }
    body["page_token"] = token.into();
    let response: serde_json::Value = test::read_body_json(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&body)
                .to_request(),
        )
        .await,
    )
    .await;
    assert_eq!(response["data"][0]["id"], "second");
    assert_eq!(response["next_page_token"], "");
    body.as_object_mut().unwrap().remove("consistency");
    body.as_object_mut().unwrap().remove("page_token");
    let response: serde_json::Value = test::read_body_json(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&body)
                .to_request(),
        )
        .await,
    )
    .await;
    assert!(
        token_json(response["next_page_token"].as_str().unwrap())["snapshot"].is_string(),
        "REST defaults still pin snapshots"
    );
    valuation.task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[actix_web::test]
async fn gift_query_schema_matches_flat_wire_filters_and_required_identities() {
    use market_gifts::{GiftFilters, GiftHistoryQuery, GiftOrdersQuery};
    let schema = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let filters = &schema["components"]["schemas"]["GiftFilters"];
    assert_eq!(filters["type"], "object");
    assert_eq!(filters["additionalProperties"], false);
    assert!(
        filters.get("allOf").is_none(),
        "a closed flattened criteria schema rejects identity fields"
    );
    let wire = serde_json::to_value(GiftOrdersQuery::new(GiftFilters::new(
        "gift".into(),
        "GRAM".into(),
    )))
    .unwrap();
    let properties = filters["properties"].as_object().unwrap();
    let keys: std::collections::BTreeSet<_> = wire["filters"].as_object().unwrap().keys().collect();
    assert_eq!(keys, properties.keys().collect());
    let required = filters["required"].as_array().unwrap();
    for key in ["collection_id", "currency_id"] {
        assert!(required.contains(&serde_json::json!(key)));
    }
    for example in [
        serde_json::json!({"filters":{"collection_id":"gift","currency_id":"GRAM"}}),
        serde_json::json!({"filters":{"collection_id":"gift","currency_id":"GRAM","models":["model"],"number":{"min":1}},"side":"bid"}),
    ] {
        let parsed: GiftOrdersQuery = serde_json::from_value(example).unwrap();
        assert!(
            serde_json::from_value::<GiftOrdersQuery>(serde_json::to_value(parsed).unwrap())
                .is_ok()
        );
    }
    for example in [
        serde_json::json!({}),
        serde_json::json!({"filters":{"collection_id":"gift"}}),
        serde_json::json!({"filters":{"currency_id":"GRAM"}}),
    ] {
        assert!(serde_json::from_value::<GiftOrdersQuery>(example).is_err());
    }
    assert!(serde_json::from_value::<GiftHistoryQuery>(serde_json::json!({"filters":{"collection_id":"gift","currency_id":"GRAM","mode":"ask"}})).is_err());
    assert!(serde_json::from_value::<GiftOrdersQuery>(serde_json::json!({"filters":{"collection_id":"gift","currency_id":"GRAM"},"side":"executed"})).is_err());
}

#[actix_web::test]
async fn rest_snapshot_pagination_reuses_retained_views_across_http_requests() {
    let (state, runtime, peer, stop) = setup().await;
    let mut fixture = page("1");
    fixture.cursor = "pagination".into();
    let mut second = fixture.entities.last().unwrap().clone();
    if let Some(w::entity::Entity::Order(order)) = &mut second.entity {
        order.id = "second".into();
        order.price = "2".into();
    }
    fixture.entities.push(second);
    peer.page.send_replace(fixture);
    tokio::time::timeout(Duration::from_secs(5), async {
        while state
            .orders(serde_json::from_value(query()).unwrap())
            .await
            .unwrap()
            .cursor
            != "pagination"
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(configure),
    )
    .await;
    let mut request = query();
    request["limit"] = 1.into();
    let first: serde_json::Value = test::read_body_json(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&request)
                .to_request(),
        )
        .await,
    )
    .await;
    assert!(!first["next_page_token"].as_str().unwrap().is_empty());
    request["page_token"] = first["next_page_token"].clone();
    let second = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/orders/query")
            .set_json(request)
            .to_request(),
    )
    .await;
    assert_eq!(second.status(), 200);
    let second: serde_json::Value = test::read_body_json(second).await;
    assert_eq!(second["data"][0]["id"], "second");
    assert_eq!(second["cursor"], first["cursor"]);
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

#[actix_web::test]
#[ignore = "requires the production frontend build and Playwright Chromium"]
async fn developer_docs_render_rest_and_websocket_console_with_local_assets() {
    let (_state, runtime, _peer, stop) = setup().await;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let server = api_server(&runtime, root.join("web/dist"));
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let task = tokio::spawn(server.run(api_stop.child_token()));
    let result = tokio::process::Command::new("node")
        .arg("scripts/docs-browser.mjs")
        .current_dir(root.join("web"))
        .env("MARKET_DOCS_TEST_URL", format!("http://{address}"))
        .output()
        .await
        .unwrap();
    api_stop.cancel();
    task.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[actix_web::test]
async fn catalog_notifications_match_http_etags_and_ignore_price_changes() {
    let (state, runtime, peer, stop) = setup().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(configure),
    )
    .await;
    let web_dir = peer._directory.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<html></html>").unwrap();
    let server = api_server(&runtime, web_dir);
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    socket.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::json!({"type":"watch_catalog","id":"catalog","collection_ids":["tg.gift.123","missing"]}).to_string().into())).await.unwrap();
    async fn receive(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> serde_json::Value {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                    socket.next().await
                {
                    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if frame["type"] == "catalog" {
                        return frame["versions"].clone();
                    }
                    assert_ne!(frame["type"], "error", "{frame}");
                }
            }
        })
        .await
        .unwrap()
    }
    let first = receive(&mut socket).await;
    assert!(first["collections"]["missing"].is_null());
    for (path, revision) in [
        ("/catalog", &first["directory"]),
        (
            "/collections/tg.gift.123",
            &first["collections"]["tg.gift.123"],
        ),
    ] {
        let response =
            test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
        assert_eq!(response.status(), 200);
        let etag = response
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(etag, format!("\"{}\"", revision.as_str().unwrap()));
        let cached = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(path)
                .insert_header(("if-none-match", etag))
                .to_request(),
        )
        .await;
        assert_eq!(cached.status(), 304);
        assert!(test::read_body(cached).await.is_empty());
    }
    peer.page.send_replace(page("2.000000002"));
    let unexpected_catalog = tokio::time::timeout(Duration::from_millis(350), async {
        loop {
            if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                socket.next().await
            {
                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                if frame["type"] == "catalog" {
                    return frame;
                }
            }
        }
    })
    .await;
    assert!(
        unexpected_catalog.is_err(),
        "Price updates must not emit catalog invalidations"
    );
    let mut renamed = page("2.000000002");
    renamed.cursor = "catalog-renamed".into();
    for entity in &mut renamed.entities {
        if let Some(w::entity::Entity::ItemType(item)) = &mut entity.entity {
            item.name = "Renamed gift".into();
        }
    }
    peer.page.send_replace(renamed);
    let changed = receive(&mut socket).await;
    assert_ne!(changed["directory"], first["directory"]);
    assert_ne!(
        changed["collections"]["tg.gift.123"],
        first["collections"]["tg.gift.123"]
    );
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/collections/tg.gift.123")
            .to_request(),
    )
    .await;
    assert_eq!(
        response.headers().get("etag").unwrap().to_str().unwrap(),
        format!(
            "\"{}\"",
            changed["collections"]["tg.gift.123"].as_str().unwrap()
        )
    );
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["collection"]["name"], "Renamed gift");
    socket.close(None).await.unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

fn valuation_page(price: &str) -> w::Page {
    let mut page = page(price);
    page.entities.push(w::Entity {
        entity: Some(w::entity::Entity::AttributeOption(w::AttributeOption {
            id: "model".into(),
            attribute_id: "tg.model".into(),
            name: "Model".into(),
            ..Default::default()
        })),
    });
    for entity in &mut page.entities {
        match entity.entity.as_mut() {
            Some(w::entity::Entity::ItemType(collection)) => {
                collection.attributes.insert(
                    "tg.model".into(),
                    w::AttributeRule {
                        required: true,
                        allowed_values: Some(w::AllowedAttributeValues {
                            ids: vec!["model".into()],
                        }),
                    },
                );
            }
            Some(w::entity::Entity::Order(order)) => {
                order.attributes.insert(
                    "tg.model".into(),
                    w::AttributePredicate {
                        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId("model".into())),
                        })),
                    },
                );
            }
            _ => {}
        }
    }
    page
}
#[actix_web::test]
async fn valuation_http_returns_the_shared_mark_and_refreshes_after_market_changes() {
    use market_gifts::valuation::{ModelLookup, ModelPosition, ValuationRequest};
    let (state, runtime, peer, stop) = setup().await;
    let price = "1.000000000000000001";
    peer.page.send_replace(valuation_page(price));
    let request = ValuationRequest {
        strategy: market_gifts::valuation::StrategyId::model_floor(),
        positions: vec![ModelPosition {
            model: ModelLookup::Catalog {
                collection: "tg.gift.123".into(),
                model: "model".into(),
            },
            quantity: 3,
        }],
        currency_id: "GRAM".into(),
    };
    let original = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(mark) = state.valuation_mark(request.clone()).await
                && mark.revision == format!("checkpoint.{price}")
            {
                break mark;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(original.total.as_str(), "3.000000000000000003");
    // Another API instance and a direct actor caller use the same normalized key and immutable mark.
    let other = MarketApi::new(runtime.data.clone(), runtime.valuation.clone());
    assert!(Arc::ptr_eq(
        &original,
        &other.valuation_mark(request.clone()).await.unwrap()
    ));
    assert!(Arc::ptr_eq(
        &original,
        &runtime.valuation.mark(request.clone()).await.unwrap()
    ));
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(other))
            .configure(configure),
    )
    .await;
    let mut duplicate = request.clone();
    duplicate.positions[0].quantity = 1;
    let mut extra = duplicate.positions[0].clone();
    extra.quantity = 2;
    duplicate.positions.push(extra);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/portfolio/value")
            .set_json(&duplicate)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let json: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(json.as_object().unwrap().len(), 4);
    assert_eq!(json["data"]["total"], "3.000000000000000003");
    assert!(json.get("total").is_none());
    assert_eq!(json["revision"], original.revision);
    assert!(json["data"].get("cursor").is_none());
    assert_eq!(json["data"]["priced"], 3);
    assert_eq!(json["data"]["unpriced"], 0);
    assert_eq!(json["data"]["currency_id"], "GRAM");
    assert_eq!(json["data"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(
        json["evaluated_at"]
            .as_str()
            .unwrap()
            .parse::<jiff::Timestamp>()
            .unwrap(),
        original.evaluated_at
    );
    peer.page
        .send_replace(valuation_page("2.000000000000000001"));
    let refreshed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let response = test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/portfolio/value")
                    .set_json(&request)
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), 200);
            let mark: serde_json::Value = test::read_body_json(response).await;
            if mark["revision"] != "checkpoint.1.000000000000000001" {
                break mark;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(refreshed["data"]["total"], "6.000000000000000003");
    assert!(refreshed["data"].get("cursor").is_none());
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
#[actix_web::test]
async fn valuation_schema_describes_mark_metadata_and_http_errors() {
    let schema = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let response = &schema["paths"]["/portfolio/value"]["post"]["responses"];
    assert_eq!(
        response["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/ValuationMark"
    );
    for code in ["400", "422", "429", "503"] {
        assert!(response[code].is_object());
    }
    let mark = &schema["components"]["schemas"]["ValuationMark"];
    for field in ["strategy", "data", "revision", "evaluated_at"] {
        assert!(
            mark["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == field)
        );
    }
    assert_eq!(mark["properties"]["evaluated_at"]["format"], "date-time");
    assert_eq!(mark["properties"]["evaluated_at"]["type"], "string");
    assert_eq!(
        mark["properties"]["data"]["$ref"],
        "#/components/schemas/Valuation"
    );
    assert!(schema["components"]["schemas"]["ApiError"]["properties"]["details"].is_object());
    assert!(
        schema["components"]["schemas"]["ValuationErrorDetails"]["properties"]["context"]
            .is_object()
    );
    assert!(
        schema["components"]["schemas"]["ValuationRequest"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "strategy")
    );
    assert!(
        schema["components"]["schemas"]
            .get("ValuationCoverage")
            .is_none()
    );
}
#[actix_web::test]
async fn valuation_admission_timeout_and_shutdown_are_http_errors_without_affecting_other_reads() {
    use actix::{ActorFutureExt, WrapFuture};
    use market_gifts::valuation::{PortfolioPositions, Valuation};
    struct Blocked {
        life: market_valuation::Lifecycle,
        started: Arc<AtomicUsize>,
    }
    impl actix::Actor for Blocked {
        type Context = actix::Context<Self>;
        fn started(&mut self, ctx: &mut Self::Context) {
            self.life.started(ctx);
        }
    }
    impl
        actix::Handler<
            market_valuation::Evaluate<
                PortfolioPositions,
                Valuation,
                market_actors::error::DataError,
            >,
        > for Blocked
    {
        type Result = actix::AtomicResponse<
            Self,
            Result<Arc<market_valuation::Mark<Valuation>>, market_actors::error::DataError>,
        >;
        fn handle(
            &mut self,
            message: market_valuation::Evaluate<
                PortfolioPositions,
                Valuation,
                market_actors::error::DataError,
            >,
            _: &mut Self::Context,
        ) -> Self::Result {
            let (_, request) = message.into_parts();
            let stop = self.life.stop_token();
            self.started.fetch_add(1, Ordering::SeqCst);
            actix::AtomicResponse::new(Box::pin(
                async move {
                    let _request = request;
                    tokio::select! {_=stop.cancelled()=>{},_=_request.cancelled.cancelled()=>{}}
                    Err(market_actors::error::DataError::unavailable("Cancelled"))
                }
                .into_actor(self)
                .map(|result, _, _| result),
            ))
        }
    }
    let (_, runtime, _, stop) = setup().await;
    let started = Arc::new(AtomicUsize::new(0));
    let valuation = market_valuation::Service::start(
        "model_floor",
        market_valuation::Config {
            max_requests: 1,
            request_timeout: Duration::from_millis(150),
        },
        |life| Blocked {
            life,
            started: started.clone(),
        },
    )
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(MarketApi::new(
                runtime.data.clone(),
                market_valuation::ValuationHandle::new([valuation.handle.clone()]).unwrap(),
            )))
            .configure(configure),
    )
    .await;
    let body = serde_json::json!({"strategy":"model_floor","positions":[],"currency_id":"GRAM"});
    let (timed_out, busy) = tokio::join!(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/portfolio/value")
                .set_json(&body)
                .to_request()
        ),
        async {
            while started.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/portfolio/value")
                    .set_json(&body)
                    .to_request(),
            )
            .await
        }
    );
    assert_eq!(busy.status(), 429);
    let json: serde_json::Value = test::read_body_json(busy).await;
    assert_eq!(json["error"]["code"], "capacity_exceeded");
    assert_eq!(timed_out.status(), 503);
    let json: serde_json::Value = test::read_body_json(timed_out).await;
    assert_eq!(json["error"]["code"], "timed_out");
    assert_eq!(json["error"]["message"], "Valuation request timed out");
    valuation.task.shutdown().await.unwrap();
    let stopped = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/portfolio/value")
            .set_json(body)
            .to_request(),
    )
    .await;
    assert_eq!(stopped.status(), 503);
    assert_eq!(
        test::call_service(&app, test::TestRequest::get().uri("/sync").to_request())
            .await
            .status(),
        200
    );
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

#[actix_web::test]
async fn named_strategies_keep_independent_marks_and_explain_missing_data() {
    use market_gifts::valuation::{ModelLookup, ModelPosition, ValuationRequest};
    let (state, runtime, peer, stop) = setup().await;
    let mut data = valuation_page("10");
    data.cursor = "strategies".into();
    let mut cheap = data
        .entities
        .iter()
        .find_map(|e| match &e.entity {
            Some(w::entity::Entity::Order(o)) => Some(o.clone()),
            _ => None,
        })
        .unwrap();
    cheap.id = "cheap".into();
    cheap.price = "3".into();
    cheap.attributes.clear(); // A valid collection-wide ask, not a model-specific quote.
    data.entities.push(w::Entity {
        entity: Some(w::entity::Entity::Order(cheap)),
    });
    peer.page.send_replace(data);
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime
            .data
            .orders(market_gifts::GiftOrdersQuery::new(
                market_gifts::GiftFilters::new("tg.gift.123".into(), "GRAM".into()),
            ))
            .await
            .unwrap()
            .cursor
            != "strategies"
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let request = ValuationRequest {
        strategy: market_gifts::valuation::StrategyId::model_floor(),
        currency_id: "GRAM".into(),
        positions: vec![ModelPosition {
            model: ModelLookup::named("Gift", "Model").unwrap(),
            quantity: 2,
        }],
    };
    let mut collection = request.clone();
    collection.strategy = market_gifts::valuation::StrategyId::collection_floor();
    let (model, broad) = tokio::join!(
        runtime.valuation.mark(request.clone()),
        runtime.valuation.mark(collection.clone()),
    );
    let model = model.unwrap();
    let broad = broad.unwrap();
    assert_eq!(model.strategy, "model_floor");
    assert_eq!(broad.strategy, "collection_floor");
    assert_eq!(model.total.as_str(), "20");
    assert_eq!(broad.total.as_str(), "6");
    assert!(Arc::ptr_eq(
        &model,
        &runtime.valuation.mark(request.clone()).await.unwrap()
    ));
    assert!(Arc::ptr_eq(
        &broad,
        &runtime.valuation.mark(collection.clone()).await.unwrap()
    ));

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(configure),
    )
    .await;
    let missing = ModelPosition {
        model: ModelLookup::named("Gift", "Missing").unwrap(),
        quantity: 1,
    };
    let mut no_data = request.clone();
    no_data.positions = vec![missing.clone()];
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/portfolio/value")
            .set_json(&no_data)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 422);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["error"]["code"], "not_enough_data");
    let detail = &body["error"]["details"];
    assert_eq!(detail["strategy"], "model_floor");
    assert_eq!(detail["currency_id"], "GRAM");
    assert_eq!(detail["revision"], "strategies");
    assert_eq!(detail["required_samples"], 1);
    assert_eq!(detail["available_samples"], 0);
    assert_eq!(
        detail["context"]["unpriced"][0]["unpriced"],
        "unknown_model"
    );
    no_data.strategy = collection.strategy.clone();
    assert_eq!(
        runtime
            .valuation
            .mark(no_data)
            .await
            .unwrap()
            .total
            .as_str(),
        "3"
    );

    let mut partial = request.clone();
    partial.positions.push(missing);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/portfolio/value")
            .set_json(partial)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["strategy"], "model_floor");
    assert_eq!(body["data"]["priced"], 2);
    assert_eq!(body["data"]["unpriced"], 1);
    for (strategy, status, code) in [
        ("unknown", 400, "unsupported_strategy"),
        ("Invalid!", 400, "invalid_query"),
    ] {
        let body = serde_json::json!({"strategy":strategy,"positions":[],"currency_id":"GRAM"});
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/portfolio/value")
                .set_json(body)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), status);
        let body: serde_json::Value = test::read_body_json(response).await;
        assert_eq!(body["error"]["code"], code);
    }
    let empty = ValuationRequest {
        positions: vec![],
        ..request
    };
    assert_eq!(
        runtime.valuation.mark(empty).await.unwrap().total.as_str(),
        "0"
    );
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

fn multiplier_page(traits: usize) -> w::Page {
    let mut data = valuation_page("6");
    data.cursor = format!("multiplier-{traits}");
    // Rebuild the order population so all evidence counts are deterministic.
    data.entities
        .retain(|e| !matches!(e.entity, Some(w::entity::Entity::Order(_))));
    for (id, attribute) in [
        ("other", "tg.model"),
        ("b", "tg.backdrop"),
        ("base", "tg.backdrop"),
    ] {
        data.entities.push(w::Entity {
            entity: Some(w::entity::Entity::AttributeOption(w::AttributeOption {
                id: id.into(),
                attribute_id: attribute.into(),
                name: id.into(),
                ..Default::default()
            })),
        });
    }
    for e in &mut data.entities {
        if let Some(w::entity::Entity::ItemType(item)) = &mut e.entity {
            for (attr, ids) in [
                ("tg.model", vec!["model", "other"]),
                ("tg.backdrop", vec!["b", "base"]),
            ] {
                item.attributes.insert(
                    attr.into(),
                    w::AttributeRule {
                        required: true,
                        allowed_values: Some(w::AllowedAttributeValues {
                            ids: ids.into_iter().map(str::to_owned).collect(),
                        }),
                    },
                );
            }
        }
    }
    for (prefix, count, price, model, backdrop) in [
        ("base", 3, 2, "other", "base"),
        ("trait", traits, 6, "model", "b"),
    ] {
        for index in 0..count {
            let mut order = w::Order {
                id: if prefix == "base" && index == 0 {
                    "order".into()
                } else {
                    format!("{prefix}-{index}")
                },
                book: Some(book()),
                mode: w::OrderMode::Ask as i32,
                price: (price + index).to_string(),
                quantity: "1".into(),
                ..Default::default()
            };
            for (attr, id) in [("tg.model", model), ("tg.backdrop", backdrop)] {
                order.attributes.insert(
                    attr.into(),
                    w::AttributePredicate {
                        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId(id.into())),
                        })),
                    },
                );
            }
            data.entities.push(w::Entity {
                entity: Some(w::entity::Entity::Order(order)),
            });
        }
    }
    data
}
#[actix_web::test]
async fn valuation_queries_use_explicit_typed_contracts_and_strategy_specific_errors() {
    let (state, runtime, peer, stop) = setup().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(configure),
    )
    .await;
    for endpoint in ["/portfolio/value", "/valuations/query"] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri(endpoint)
                .set_json(serde_json::json!({"positions":[],"currency_id":"GRAM"}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);
    }
    for count in [1, 3] {
        peer.page.send_replace(multiplier_page(count));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let page = runtime
                    .data
                    .orders(market_gifts::GiftOrdersQuery::new(
                        market_gifts::GiftFilters::new("tg.gift.123".into(), "GRAM".into()),
                    ))
                    .await
                    .unwrap();
                if page.cursor == format!("multiplier-{count}") {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        for (strategy, field, id) in [
            ("model_multiplier", "model_id", "model"),
            ("backdrop_multiplier", "backdrop_id", "b"),
        ] {
            let query = serde_json::json!({"strategy":strategy,"collection_id":"tg.gift.123",field:id,"currency_id":"GRAM"});
            let response = test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/valuations/query")
                    .set_json(&query)
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), if count == 1 { 422 } else { 200 });
            let body: serde_json::Value = test::read_body_json(response).await;
            if count == 1 {
                assert_eq!(body["error"]["code"], "not_enough_data");
                let d = &body["error"]["details"];
                assert_eq!(d["strategy"], strategy);
                assert_eq!(d["available_samples"], 1);
                assert_eq!(d["required_samples"], 3);
                assert_eq!(d["context"]["kind"], "multiplier");
                assert_eq!(d["context"]["trait_samples"], 1);
                assert!(d["context"].get("unpriced").is_none());
            } else {
                assert_eq!(body["strategy"], strategy);
                assert_eq!(body["data"][field], id);
                assert_eq!(body["data"]["factor"], "3");
                assert_eq!(body["data"]["evidence"]["collection"]["samples"], 6);
                assert_eq!(body["data"]["evidence"]["selected_trait"]["samples"], 3);
                assert!(body.get("coverage").is_none());
                assert!(body["data"].get("total").is_none());
                let again = test::call_service(
                    &app,
                    test::TestRequest::post()
                        .uri("/valuations/query")
                        .set_json(query)
                        .to_request(),
                )
                .await;
                let again: serde_json::Value = test::read_body_json(again).await;
                assert_eq!(body["evaluated_at"], again["evaluated_at"]);
            }
        }
    }
    let schema = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let variants = schema["components"]["schemas"]["StrategyMark"]["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(variants.len(), 4);
    for (variant, name) in variants.iter().zip([
        "model_floor",
        "collection_floor",
        "model_multiplier",
        "backdrop_multiplier",
    ]) {
        let tag = variant["allOf"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|part| part["properties"].get("strategy"))
            .unwrap();
        assert_eq!(tag["enum"][0], name);
    }
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
