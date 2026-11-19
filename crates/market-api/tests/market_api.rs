use actix_web::{App, test, web};
use futures_util::{SinkExt, StreamExt};
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
    reads: market_replica::ReadRuntime,
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
        let reads =
            market_replica::ReadRuntime::new(replica.reader, 8, stop.child_token()).unwrap();
        let data = MarketDataService::start_with_shutdown(reads.handle(), 8, stop.child_token())
            .await
            .unwrap();
        let (registry, valuation_task) =
            market_valuation::ValuationHandle::start(&data.arbiter(), reads.handle())
                .await
                .unwrap();
        let valuation_stop = stop.child_token();
        let jobs = vec![
            tokio::spawn(data.task.run(stop.child_token())),
            tokio::spawn(async move {
                valuation_task
                    .run(valuation_stop)
                    .await
                    .map_err(|e| Status::unavailable(e.to_string()))
            }),
            tokio::spawn(replica.replication.run(hub, stop.child_token())),
        ];
        Self {
            data: data.handle,
            reads,
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
        self.reads.shutdown().await;
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
    serde_json::json!({"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","limit":25})
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
    assert_eq!(defaults["data"][0]["kind"], "ask");
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
    assert_eq!(
        test::call_service(&app, test::TestRequest::get().uri("/sync").to_request())
            .await
            .status(),
        404
    );
    for path in ["/healthz", "/readyz"] {
        assert_eq!(
            test::call_service(&app, test::TestRequest::get().uri(path).to_request())
                .await
                .status(),
            200
        );
    }
    assert!(body.get("sync").is_none());
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
    assert!(schema["components"]["schemas"]["OrderTargetResponse"].is_object());
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
    for path in ["/catalog", "/collections/{id}"] {
        assert!(schema["paths"][path].is_object());
    }
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
#[actix_web::test]
async fn public_websocket_queries_share_local_windows_and_one_replication_stream() {
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
        let initial = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                    socket.next().await
                {
                    break serde_json::from_str::<serde_json::Value>(&text).unwrap();
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            initial,
            serde_json::json!({"type":"market.status","status":"available"})
        );

        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({"type":"orders.watch","id":"offers","params":{"query":slice_query()}})
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
                    if is_order_event(&value) && event_rows(&value)[0]["price"] == price {
                        assert_eq!(
                            event_rows(&value)[0]["target"]["gift"]["id"],
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
    assert!(
        tokio::time::timeout(Duration::from_millis(150), async {
            loop {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                    a.next().await
                {
                    break text;
                }
            }
        })
        .await
        .is_err(),
        "checkpoint-only changes must not emit market status"
    );
    peer.disconnect.cancel();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = a.next().await {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "market.status" && value["status"] == "delayed" {
                    assert!(value.get("page").is_none());
                    assert!(value.get("revision").is_none());
                    assert!(value.get("id").is_none());
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
    let schema: serde_json::Value = serde_json::from_str(&json).unwrap();
    let properties = &schema["components"]["schemas"]["OrderResponse"]["properties"];
    assert!(properties.get("mode").is_none());
    assert!(properties.get("parent_order_id").is_none());
    assert!(properties.get("kind").is_some());
    assert_eq!(properties["quantity"]["minimum"], 1);
    let page_ref = serde_json::json!({"$ref":"#/components/schemas/OrderReadResponse"});
    {
        let path = "/orders/query";
        assert_eq!(
            schema["paths"][path]["post"]["responses"]["200"]["content"]["application/json"]["schema"],
            page_ref
        );
    }
    for (path, method) in [
        ("/orders/query", "post"),
        ("/orders/query", "post"),
        ("/catalog", "get"),
        ("/collections/{id}", "get"),
    ] {
        for status in ["429", "503"] {
            assert!(
                schema["paths"][path][method]["responses"]
                    .get(status)
                    .is_some(),
                "{method} {path} must document {status}"
            );
        }
    }
    for path in ["/sync", "/readyz", "/healthz"] {
        assert!(schema["paths"].get(path).is_none());
    }
    assert!(schema["components"]["schemas"].get("SyncStatus").is_none());
    assert!(schema["components"]["schemas"].get("SyncState").is_none());
    assert!(
        schema["paths"]["/orders/query"]["delete"]["responses"]
            .get("503")
            .is_some()
    );
    assert!(schema.get("x-read-limits").is_none());
    assert!(schema["paths"].get("/capabilities").is_none());
    assert!(schema["paths"].get("/orders/window").is_none());
    assert!(
        schema["components"]["schemas"]
            .get("OrderWindowResponse")
            .is_none()
    );
    let frame = schema["components"]["schemas"]["ServerFrame"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variant| variant.to_string().contains("OrderReadResponse"))
        .unwrap();
    assert!(frame.to_string().contains("OrderReadResponse"));

    for name in [
        "ValuationErrorContext",
        "ModelValue",
        "UnpricedReason",
        "PortfolioPositions",
        "ValuationRequest",
        "BookKey",
        "AttributeValue",
        "Predicate",
        "Scope",
        "CatalogEntry",
        "GiftOrder",
        "GiftTarget",
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
async fn oversized_order_batches_report_terminal_errors_and_release_the_id() {
    let (_, runtime, _, stop) = setup().await;
    let server = ApiServer::bind(
        ApiConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            workers: 1,
            limits: market_api::ApiLimits {
                websocket: market_api::WebSocketLimits {
                    order_bytes: 64,
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
                serde_json::json!({"type":"orders.watch","id":"offers","params":{"query":slice_query()}})
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
                .contains("byte budget")
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

fn slug_page(samples: usize) -> w::Page {
    let mut page = multiplier_page(samples);
    for (id, kind) in [
        ("tg.number", w::AttributeType::Integer),
        ("tg.model", w::AttributeType::Enum),
        ("tg.backdrop", w::AttributeType::Enum),
    ] {
        page.entities.push(w::Entity {
            entity: Some(w::entity::Entity::Attribute(w::Attribute {
                id: id.into(),
                name: id.into(),
                value_type: kind as i32,
                ..Default::default()
            })),
        });
    }
    for row in &mut page.entities {
        match row.entity.as_mut() {
            Some(w::entity::Entity::ItemType(item)) => {
                item.aliases.push(w::ExternalId {
                    namespace: "tg.slug".into(),
                    id: "Gift".into(),
                });
                item.attributes
                    .insert("tg.number".into(), w::AttributeRule::default());
            }
            Some(w::entity::Entity::Order(order)) => {
                let selected = order.id == "trait.0" || order.id == "trait-0";
                // Every order is a concrete, unique collectible. Only one is Gift-123.
                let number = if selected { 123 } else { 999 };
                order.item_id = Some(if selected {
                    "gift-123".into()
                } else {
                    format!("gift-{}", order.id)
                });
                order.attributes.insert(
                    "tg.number".into(),
                    w::AttributePredicate {
                        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                            value: Some(w::attribute_value::Value::Integer(number)),
                        })),
                    },
                );
            }
            _ => {}
        }
    }
    page
}
async fn wait_slug(state: &MarketApi) -> std::sync::Arc<market_valuation::GiftFloorMark> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(value) = state
                .gift_valuation(market_valuation::GiftValuationRequest {
                    gift: market_gifts::GiftSlug::new("Gift-123").unwrap().into(),
                    currency_id: "GRAM".into(),
                })
                .await
            {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[actix_web::test]
async fn gift_valuation_returns_one_coherent_report_and_partial_errors() {
    let (state, runtime, peer, stop) = setup().await;
    peer.page.send_replace(slug_page(3));
    let direct = wait_slug(&state).await;
    assert_eq!(direct.strategy, "gift_floor");
    assert_eq!(direct.collection_floor.value().unwrap().as_str(), "2");
    assert_eq!(direct.model_floor.value().unwrap().as_str(), "6");
    assert_eq!(direct.backdrop_floor.value().unwrap().as_str(), "6");
    assert_eq!(direct.model_backdrop_floor.value().unwrap().as_str(), "6");
    assert_eq!(direct.model_multiplier.value().unwrap().as_str(), "3");
    assert_eq!(direct.backdrop_multiplier.value().unwrap().as_str(), "3");
    assert_eq!(direct.final_mark.value().unwrap().as_str(), "6");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(configure),
    )
    .await;
    let request = serde_json::json!({"gift":"https://t.me/nft/Gift-123","currency_id":"GRAM"});
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/valuations/gift")
            .set_json(&request)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["slug"], "Gift-123");
    assert_eq!(body["gift"]["number"], 123);
    assert_eq!(body["final_mark"]["value"], "6");
    let fields = body["display"]["fields"].as_array().unwrap();
    assert_eq!(fields.len(), 7);
    for (id, attribute) in [
        ("model_floor", "model"),
        ("model_multiplier", "model"),
        ("backdrop_floor", "backdrop"),
        ("backdrop_multiplier", "backdrop"),
    ] {
        assert_eq!(
            fields.iter().find(|field| field["id"] == id).unwrap()["attribute"],
            attribute
        );
    }
    let final_field = fields
        .iter()
        .find(|field| field["id"] == "final_mark")
        .unwrap();
    assert_eq!(final_field["primary"], true);
    assert_eq!(final_field["value"]["kind"], "currency");
    assert_eq!(final_field["value"]["amount"], "6.00");
    assert_eq!(final_field["value"]["currency_id"], "GRAM");
    assert!(body.get("revision").is_none());
    assert!(body.get("sync").is_none());
    assert_eq!(body["stale"], false);
    assert!(body["evaluated_at"].is_string());
    assert!(body.get("strategies").is_none());
    assert!(body.get("data").is_none());
    // Both transports share the actor and public presentation, including partial metric errors.
    let web_dir = peer._directory.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "fixture").unwrap();
    let server = api_server(&runtime, web_dir);
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    socket.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::json!({"type":"request","id":"value-1","method":"valuation.evaluate","params":request}).to_string().into())).await.unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                socket.next().await
            {
                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                if frame["type"] == "market.status" {
                    assert!(frame.get("id").is_none());
                    continue;
                }
                break frame;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(frame["type"], "result");
    assert_eq!(frame["id"], "value-1");
    assert_eq!(frame["method"], "valuation.evaluate");
    let mut received = frame["data"].clone();
    let mut expected = body.clone();
    received.as_object_mut().unwrap().remove("evaluated_at");
    expected.as_object_mut().unwrap().remove("evaluated_at");
    assert_eq!(received, expected);
    let missing =
        serde_json::json!({"gift":{"collection_id":"unknown","number":123},"currency_id":"GRAM"});
    let http_error: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/valuations/gift")
            .set_json(&missing)
            .to_request(),
    )
    .await;
    socket.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::json!({"type":"request","id":"value-2","method":"valuation.evaluate","params":missing}).to_string().into())).await.unwrap();
    let error_frame = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                socket.next().await
            {
                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                if frame["type"] == "market.status" {
                    assert!(frame.get("id").is_none());
                    continue;
                }
                break frame;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(error_frame["id"], "value-2");
    assert_eq!(error_frame["type"], "error");
    assert_eq!(error_frame["error"], http_error["error"]);
    socket.close(None).await.unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    for route in ["/portfolio/value", "/valuations/query"] {
        assert_eq!(
            test::call_service(
                &app,
                test::TestRequest::post()
                    .uri(route)
                    .set_json(&request)
                    .to_request()
            )
            .await
            .status(),
            404
        );
    }
    let mut partial = slug_page(3);
    partial.cursor = "partial".into();
    for row in &mut partial.entities {
        if let Some(w::entity::Entity::Order(order)) = &mut row.entity {
            order.attributes.remove("tg.backdrop");
        }
    }
    peer.page.send_replace(partial);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let report = wait_slug(&state).await;
            if report.revision == "partial" {
                assert!(report.gift.traits.backdrop.is_none());
                assert!(report.model_floor.value().is_some());
                assert!(report.backdrop_floor.value().is_none());
                assert!(report.final_mark.value().is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/valuations/gift")
            .set_json(&request)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let report: serde_json::Value = test::read_body_json(response).await;
    assert!(report.get("revision").is_none());
    assert!(report.get("sync").is_none());
    assert_eq!(
        report["backdrop_multiplier"]["error"]["code"],
        "missing_trait"
    );
    assert_eq!(report["final_mark"]["error"]["code"], "not_enough_data");
    let primary = report["display"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["primary"] == true)
        .unwrap();
    assert_eq!(primary["value"]["kind"], "unavailable");
    assert_eq!(primary["value"]["label"], "Not enough data");
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
#[actix_web::test]
async fn gift_valuation_validates_links_reports_unknown_and_conflicting_gifts_and_documents_new_contract()
 {
    let (state, runtime, peer, stop) = setup().await;
    peer.page.send_replace(slug_page(3));
    wait_slug(&state).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(configure),
    )
    .await;
    for (gift, expected) in [
        ("Gift-0", 400),
        ("https://evil.example/nft/Gift-123", 400),
        ("Unknown-123", 404),
        ("Gift-789", 404),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/valuations/gift")
                .set_json(serde_json::json!({"gift":gift,"currency_id":"GRAM"}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), expected, "{gift}");
    }
    let schema: serde_json::Value =
        serde_json::from_str(&market_api::schema_json().unwrap()).unwrap();
    assert!(schema["paths"].get("/portfolio/value").is_none());
    assert!(schema["paths"].get("/valuations/query").is_none());
    assert!(
        schema["paths"]["/valuations/gift"]["post"]["responses"]
            .get("404")
            .is_some()
    );
    assert!(
        schema["components"]["schemas"]
            .get("ValuationRequest")
            .is_none()
    );
    assert!(
        schema["components"]["schemas"]
            .get("ValuationQuery")
            .is_none()
    );
    let mut conflict = slug_page(3);
    conflict.cursor = "conflict".into();
    let mut row = conflict
        .entities
        .iter()
        .find_map(|e| match &e.entity {
            Some(w::entity::Entity::Order(o)) if o.item_id.as_deref() == Some("gift-123") => {
                Some(o.clone())
            }
            _ => None,
        })
        .unwrap();
    row.id = "conflicting".into();
    row.item_id = Some("other-identity".into());
    conflict.entities.push(w::Entity {
        entity: Some(w::entity::Entity::Order(row)),
    });
    peer.page.send_replace(conflict);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let result = state
                .gift_valuation(market_valuation::GiftValuationRequest {
                    gift: market_gifts::GiftSlug::new("Gift-123").unwrap().into(),
                    currency_id: "GRAM".into(),
                })
                .await;
            if let Err(error) = result {
                assert!(matches!(
                    error.code,
                    market_api::error::ErrorCode::Unavailable
                ));
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

#[actix_web::test]
async fn gift_lookup_is_currency_independent_and_missing_evidence_remains_a_successful_report() {
    let (state, runtime, peer, stop) = setup().await;
    let mut initial = slug_page(3);
    initial.entities.push(w::Entity {
        entity: Some(w::entity::Entity::Currency(w::Currency {
            id: "XTR".into(),
            name: "Stars".into(),
            decimals: 0,
        })),
    });
    for row in &mut initial.entities {
        if let Some(w::entity::Entity::Order(order)) = &mut row.entity
            && order.item_id.as_deref() == Some("gift-123")
        {
            order.book.as_mut().unwrap().currency_id = "XTR".into();
        }
    }
    peer.page.send_replace(initial);
    let value = wait_slug(&state).await;
    assert_eq!(value.currency_id.as_str(), "GRAM");
    assert!(value.model_floor.value().is_some());
    let mut expired = slug_page(3);
    expired.cursor = "expired".into();
    for row in &mut expired.entities {
        if let Some(w::entity::Entity::Order(order)) = &mut row.entity {
            order.expires_at_ms = Some(1);
        }
    }
    peer.page.send_replace(expired);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let value = wait_slug(&state).await;
            if value.revision == "expired" {
                assert!(value.final_mark.value().is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/valuations/gift")
            .set_json(serde_json::json!({"gift":"Gift-123","currency_id":"GRAM"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = test::read_body_json(response).await;
    for key in [
        "collection_floor",
        "model_floor",
        "backdrop_floor",
        "model_backdrop_floor",
        "model_multiplier",
        "backdrop_multiplier",
        "final_mark",
    ] {
        assert_eq!(body[key]["error"]["code"], "not_enough_data", "{key}");
    }
    // A criteria ask cannot fabricate a gift identity from its number predicate.
    let mut criteria = slug_page(3);
    criteria.cursor = "criteria".into();
    for row in &mut criteria.entities {
        if let Some(w::entity::Entity::Order(order)) = &mut row.entity
            && order.item_id.as_deref() == Some("gift-123")
        {
            order.item_id = None;
        }
    }
    peer.page.send_replace(criteria);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let result = state
                .gift_valuation(market_valuation::GiftValuationRequest {
                    gift: market_gifts::GiftSlug::new("Gift-123").unwrap().into(),
                    currency_id: "GRAM".into(),
                })
                .await;
            if let Err(error) = result {
                assert!(matches!(error.code, market_api::error::ErrorCode::NotFound));
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

#[actix_web::test]
async fn canonical_slug_lookup_survives_renames_and_rejects_missing_or_ambiguous_aliases() {
    let (state, runtime, peer, stop) = setup().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(configure),
    )
    .await;
    for (case, aliases, duplicate, expected) in [
        ("renamed", vec!["Gift"], false, 200),
        ("missing", vec![], false, 404),
        ("backfilled", vec!["Gift"], false, 200),
        ("ambiguous", vec!["Gift"], true, 400),
        ("conflicting", vec!["Gift", "Other"], false, 503),
        ("malformed", vec!["Gift Name"], false, 503),
    ] {
        let mut page = slug_page(3);
        page.cursor = case.into();
        let mut second = None;
        for row in &mut page.entities {
            if let Some(w::entity::Entity::ItemType(item)) = row.entity.as_mut() {
                item.name = "A renamed collection".into();
                item.aliases.retain(|a| a.namespace != "tg.slug");
                item.aliases.extend(aliases.iter().map(|id| w::ExternalId {
                    namespace: "tg.slug".into(),
                    id: (*id).into(),
                }));
                if duplicate {
                    let mut other = item.clone();
                    other.id = "other".into();
                    other.name = "A different collection".into();
                    other.aliases.retain(|a| a.namespace != "tg");
                    other
                        .aliases
                        .iter_mut()
                        .find(|a| a.namespace == "tg.slug")
                        .unwrap()
                        .id = "gIfT".into();
                    second = Some(other);
                }
            }
        }
        if let Some(item) = second {
            page.entities.push(w::Entity {
                entity: Some(w::entity::Entity::ItemType(item)),
            });
        }
        peer.page.send_replace(page);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if runtime.reads.handle().reader().cursor().unwrap() == case {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/valuations/gift")
                .set_json(
                    serde_json::json!({"gift":"https://t.me/nft/gIfT-00123", "currency_id":"GRAM"}),
                )
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), expected, "{case}");
        if case == "missing" {
            let response = test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/valuations/gift")
                    .set_json(serde_json::json!({
                        "gift": { "collection_id": "tg.gift.123", "number": 123 },
                        "currency_id": "GRAM"
                    }))
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), 200);
            let report: serde_json::Value = test::read_body_json(response).await;
            assert!(report["slug"].is_null());
            assert_eq!(report["gift"]["id"], "gift-123");
            assert_eq!(report["final_mark"]["value"], "6");
            for (collection, number) in [("tg.gift.123", 789), ("missing", 123)] {
                let response = test::call_service(
                    &app,
                    test::TestRequest::post().uri("/valuations/gift")
                        .set_json(serde_json::json!({
                            "gift": { "collection_id": collection, "number": number }, "currency_id": "GRAM"
                        })).to_request(),
                ).await;
                assert_eq!(response.status(), 404);
            }
        }
        if matches!(case, "renamed" | "backfilled") {
            let report: serde_json::Value = test::read_body_json(response).await;
            assert_eq!(report["gift"]["number"], 123);
            assert_eq!(report["gift"]["collection_id"], "tg.gift.123");
            for path in ["/catalog", "/collections/tg.gift.123"] {
                let response =
                    test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
                let body: serde_json::Value = test::read_body_json(response).await;
                let collection = if path == "/catalog" {
                    &body["collections"][0]
                } else {
                    &body["collection"]
                };
                assert_eq!(collection["slug"], "Gift");
                assert_eq!(collection["name"], "A renamed collection");
            }
        }
    }
    let schema: serde_json::Value =
        serde_json::from_str(&market_api::schema_json().unwrap()).unwrap();
    assert_eq!(schema["components"]["schemas"]["GiftNumber"]["minimum"], 1);
    assert!(schema["components"]["schemas"]["GiftLookup"]["oneOf"].is_array());
    assert_eq!(
        schema["components"]["schemas"]["GiftNumber"]["type"],
        "integer"
    );
    assert_eq!(
        schema["components"]["schemas"]["CollectionSlug"]["type"],
        "string"
    );
    assert_eq!(
        schema["components"]["schemas"]["GiftSlug"]["type"],
        "string"
    );
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

#[actix_web::test]
async fn rest_and_websocket_share_public_order_fields_reasons_and_link_validation() {
    let (state, runtime, peer, stop) = setup().await;
    let mut source = page("1.000000001");
    source.cursor = "order-presentation".into();
    let mut history = None;
    let mut additional = None;
    for row in &mut source.entities {
        if let Some(w::entity::Entity::Order(order)) = row.entity.as_mut() {
            order
                .meta
                .insert("url".into(), "javascript:private_source_diagnostic".into());
            let mut concrete = order.clone();
            concrete.id = "concrete".into();
            concrete.price = "2".into();
            concrete.meta.insert(
                "url".into(),
                "https://t.me/market/app?startapp=gift42".into(),
            );
            let mut execution = concrete.clone();
            execution.id = "execution".into();
            execution.mode = w::OrderMode::Executed as i32;
            execution.parent_order_id = Some("internal-parent-id".into());
            execution.executed_at_ms = Some(1800000000123);
            execution.meta.insert(
                "url".into(),
                "https://private:password@example.com/secret".into(),
            );
            history = Some(execution);
            additional = Some(concrete);
            order.attributes.insert(
                "private.source.constraint".into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::Boolean(true)),
                    })),
                },
            );
        }
    }
    for order in [additional.unwrap(), history.unwrap()] {
        source.entities.push(w::Entity {
            entity: Some(w::entity::Entity::Order(order)),
        });
    }
    for (id, price) in [("exhausted-head", "0"), ("exhausted-tail", "30")] {
        source.entities.push(w::Entity {
            entity: Some(w::entity::Entity::Order(w::Order {
                id: id.into(),
                book: Some(book()),
                mode: w::OrderMode::Ask as i32,
                price: price.into(),
                quantity: "0.00".into(),
                ..Default::default()
            })),
        });
    }
    peer.page.send_replace(source);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if runtime.reads.handle().reader().cursor().unwrap() == "order-presentation" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
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
    assert_eq!(response.status(), 200);
    let rest: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(rest["data"].as_array().unwrap().len(), 2);
    assert_eq!(rest["data"][0]["kind"], "ask");
    assert_eq!(
        rest["data"][0]["target"]["reason"]["code"],
        "unsupported_constraints"
    );
    assert!(rest["data"][0]["target"]["reason"]["message"].is_string());
    assert!(rest["data"][0]["market_url"].is_null());
    assert_eq!(
        rest["data"][1]["market_url"],
        "https://t.me/market/app?startapp=gift42"
    );
    for row in rest["data"].as_array().unwrap() {
        assert!(row.get("mode").is_none());
        assert!(row.get("parent_order_id").is_none());
        assert!(row.get("observed_at").is_some());
    }
    for hidden in [
        "private.source.constraint",
        "private_source_diagnostic",
        "internal-parent-id",
    ] {
        assert!(!rest.to_string().contains(hidden));
    }
    let mut paged_query = query();
    paged_query["limit"] = 1.into();
    let first = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/orders/query")
            .set_json(&paged_query)
            .to_request(),
    )
    .await;
    assert_eq!(first.status(), 200);
    let first: serde_json::Value = test::read_body_json(first).await;
    assert_eq!(first["data"].as_array().unwrap().len(), 1);
    assert!(!first["next"].as_str().unwrap().is_empty());
    paged_query["navigation"] = serde_json::json!({"type":"after","key":first["next"]});
    let second = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/orders/query")
            .set_json(paged_query)
            .to_request(),
    )
    .await;
    assert_eq!(second.status(), 200);
    let second: serde_json::Value = test::read_body_json(second).await;
    assert_eq!(second["data"].as_array().unwrap().len(), 1);
    assert!(second["next"].is_null());
    assert_ne!(first["data"][0]["id"], second["data"][0]["id"]);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/orders/query")
            .set_json(
                serde_json::json!({"source":"history","filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"}}),
            )
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let history: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(history["data"][0]["kind"], "executed");
    assert!(history["data"][0]["executed_at"].is_string());
    assert!(history["data"][0]["market_url"].is_null());
    assert!(!history.to_string().contains("password"));
    assert!(history["data"][0].get("parent_order_id").is_none());

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
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({"type":"orders.watch","id":"orders","params":{"query":slice_query()}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let socket_page = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                socket.next().await
            {
                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_ne!(frame["type"], "error", "{frame}");
                if frame["type"] == "orders.snapshot" {
                    break serde_json::json!({"data":frame["data"],"range":frame["range"],"next":frame["next"]});
                }
            }
        }
    })
    .await
    .unwrap();
    let mut socket_data = socket_page["data"].clone();
    for row in socket_data.as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("position");
        row.as_object_mut().unwrap().remove("sort_key");
    }
    assert_eq!(socket_data, rest["data"]);
    let schema: serde_json::Value =
        serde_json::from_str(&market_api::schema_json().unwrap()).unwrap();
    let schemas = &schema["components"]["schemas"];
    assert!(schemas.get("GiftOrder").is_none());
    assert!(schemas.get("GiftTarget").is_none());
    assert!(schemas["OrderResponse"]["properties"].get("mode").is_none());
    assert!(
        schemas["OrderResponse"]["properties"]
            .get("parent_order_id")
            .is_none()
    );
    {
        let path = "/orders/query";
        assert_eq!(
            schema["paths"][path]["post"]["responses"]["200"]["content"]["application/json"]["schema"]
                ["$ref"],
            "#/components/schemas/OrderReadResponse"
        );
    }
    socket.close(None).await.unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

#[actix_web::test]
async fn atomic_range_changes_share_public_presentation_without_http_continuations() {
    let (_state, runtime, peer, stop) = setup().await;
    let web_dir = peer._directory.path().join("window-web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "fixture").unwrap();
    let server = api_server(&runtime, web_dir);
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    let command = serde_json::json!({"type":"orders.watch","id":"orders.1","params":{"query":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","sort":"price_asc","limit":50}}});
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            command.to_string().into(),
        ))
        .await
        .unwrap();
    async fn receive(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        price: &str,
    ) -> serde_json::Value {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                    socket.next().await
                {
                    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if is_order_event(&frame) && event_rows(&frame)[0]["price"] == price {
                        break frame;
                    }
                }
            }
        })
        .await
        .unwrap()
    }
    let first = receive(&mut socket, "1.000000001").await;
    assert_eq!(first["id"], "orders.1");
    assert!(first["range"]["start"].is_null());
    assert!(first["range"]["end"].is_null());
    assert!(first["data"][0]["key"].is_string());
    assert_eq!(first["data"][0]["kind"], "ask");
    assert!(first["data"][0].get("parent_order_id").is_none());
    assert!(first.get("revision").is_none());
    peer.page.send_replace(page("2.000000002"));
    let next = receive(&mut socket, "2.000000002").await;
    assert_ne!(first["data"][0]["key"], next["upserts"][0]["key"]);
    assert_eq!(first["type"], "orders.snapshot");
    assert_eq!(next["type"], "orders.changed");
    assert_eq!(
        next["sequence"].as_u64(),
        first["sequence"].as_u64().map(|n| n + 1)
    );
    assert_eq!(peer.reads.load(Ordering::SeqCst), 1);
    assert_eq!(peer.watches.load(Ordering::SeqCst), 1);
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "{\"type\":\"cancel\",\"id\":\"orders.1\"}".into(),
        ))
        .await
        .unwrap();
    socket.close(None).await.unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

fn is_order_event(frame: &serde_json::Value) -> bool {
    matches!(
        frame["type"].as_str(),
        Some("orders.snapshot" | "orders.changed")
    )
}
fn event_rows(frame: &serde_json::Value) -> &serde_json::Value {
    if frame["type"] == "orders.changed" {
        &frame["upserts"]
    } else {
        &frame["data"]
    }
}
fn slice_query() -> serde_json::Value {
    let query = query();
    serde_json::json!({"filters":query["filters"],"source":"asks","sort":query.get("sort").cloned().unwrap_or("price_asc".into()),"limit":50})
}

#[actix_web::test]
async fn server_deadline_bounds_json_extraction_without_stopping_the_service() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_state, runtime, _peer, stop) = setup().await;
    let api = ApiServer::bind(
        ApiConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            cors_origins: vec!["https://example.test".into()],
            limits: market_api::ApiLimits {
                request_timeout: Duration::from_millis(100),
                ..Default::default()
            },
            ..Default::default()
        },
        runtime.data.clone(),
        runtime.valuation.clone(),
    )
    .unwrap();
    let addr = api.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(api.run(api_stop.clone()));
    // Complete headers, but leave JSON unfinished. A header-only timeout cannot bound this.
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket.write_all(b"POST /orders/query HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nOrigin: https://example.test\r\nContent-Length: 200\r\nConnection: close\r\n\r\n{").await.unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), async {
        let mut bytes = Vec::new();
        let mut chunk = [0; 1024];
        loop {
            let len = socket.read(&mut chunk).await.unwrap();
            if len == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..len]);
            if String::from_utf8_lossy(&bytes).contains("HTTP request timed out") {
                break;
            }
        }
        String::from_utf8(bytes).unwrap()
    })
    .await
    .expect("The handler deadline must also bound payload extraction");
    assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    assert!(response.contains("\"code\":\"timed_out\""), "{response}");
    let lower = response.to_ascii_lowercase();
    assert!(
        lower.contains("access-control-allow-origin: https://example.test"),
        "{response}"
    );
    assert!(
        lower.contains("x-content-type-options: nosniff"),
        "{response}"
    );
    drop(socket);
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(bytes.starts_with(b"HTTP/1.1 200"));
    api_stop.cancel();
    job.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}

#[actix_web::test]
#[ignore = "requires Node.js 24; runs the same probe as scripts/smoke.py"]
async fn range_watch_smoke_probe_uses_portable_positions_and_frozen_leases() {
    let (state, runtime, peer, stop) = setup().await;
    let mut data = page("1");
    let original = data
        .entities
        .iter()
        .find_map(|entity| match &entity.entity {
            Some(w::entity::Entity::Order(order)) => Some(order.clone()),
            _ => None,
        })
        .unwrap();
    for i in 2..=6 {
        let mut order = original.clone();
        order.id = format!("order.{i}");
        order.price = i.to_string();
        data.entities.push(w::Entity {
            entity: Some(w::entity::Entity::Order(order)),
        });
    }
    data.cursor = "probe".into();
    peer.page.send_replace(data);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if state
                .test_orders(serde_json::from_value(query()).unwrap())
                .await
                .unwrap()
                .data
                .len()
                == 6
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let api = ApiServer::bind(
        ApiConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        },
        runtime.data.clone(),
        runtime.valuation.clone(),
    )
    .unwrap();
    let base = format!("http://{}", api.local_addr().unwrap());
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(api.run(api_stop.clone()));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let availability = tokio::process::Command::new("node")
        .arg(root.join("scripts/market-status-probe.mjs"))
        .arg(&base)
        .output()
        .await
        .unwrap();
    assert!(
        availability.status.success(),
        "{}",
        String::from_utf8_lossy(&availability.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&availability.stdout).trim(),
        "available"
    );
    let output = tokio::process::Command::new("node")
        .arg(root.join("scripts/range-probe.mjs"))
        .env("PUBLIC_URL", base)
        .env("PUBLIC_QUERY", query().to_string())
        .output()
        .await
        .unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct TestOrders {
    data: Vec<market_gifts::GiftOrder>,
}
trait QueryFixture {
    async fn test_orders(
        &self,
        query: market_gifts::GiftOrderQuery,
    ) -> Result<TestOrders, market_api::error::ApiError>;
}
impl QueryFixture for MarketApi {
    async fn test_orders(
        &self,
        query: market_gifts::GiftOrderQuery,
    ) -> Result<TestOrders, market_api::error::ApiError> {
        Ok(TestOrders {
            data: self
                .query_orders(query)
                .await?
                .result
                .data
                .into_iter()
                .map(|row| row.order)
                .collect(),
        })
    }
}
#[actix_web::test]
async fn unified_query_uses_source_defaults_one_key_and_explicit_snapshot_leases() {
    let (state, runtime, peer, stop) = setup().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(configure),
    )
    .await;
    let send = |body: serde_json::Value| {
        test::TestRequest::post()
            .uri("/orders/query")
            .set_json(body)
            .to_request()
    };
    let latest: serde_json::Value = test::call_and_read_body_json(&app, send(query())).await;
    assert_eq!(latest["data"][0]["price"], "1.000000001");
    assert!(
        latest["data"][0]["key"]
            .as_str()
            .unwrap()
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    );
    for name in [
        "result",
        "revision",
        "sync",
        "cursor",
        "next_page_token",
        "snapshot",
    ] {
        assert!(latest.get(name).is_none(), "{name}");
    }
    for name in ["position", "sort_key"] {
        assert!(latest["data"][0].get(name).is_none());
    }
    assert!(latest["next"].is_null());
    let mut frozen_query = query();
    frozen_query["view"] = serde_json::json!({"kind":"snapshot"});
    let frozen: serde_json::Value =
        test::call_and_read_body_json(&app, send(frozen_query.clone())).await;
    frozen_query["view"]["id"] = frozen["snapshot"].clone();
    peer.page.send_replace(page("2.000000002"));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if state
                .test_orders(serde_json::from_value(query()).unwrap())
                .await
                .unwrap()
                .data[0]
                .price
                .as_str()
                == "2.000000002"
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let same: serde_json::Value =
        test::call_and_read_body_json(&app, send(frozen_query.clone())).await;
    assert_eq!(same["data"], frozen["data"]);
    let row_key = frozen["data"][0]["key"].clone();
    for direction in ["after", "before"] {
        let mut next = frozen_query.clone();
        next["navigation"] = serde_json::json!({"type":direction,"key":row_key});
        let result: serde_json::Value = test::call_and_read_body_json(&app, send(next)).await;
        assert!(result["data"].as_array().unwrap().is_empty());
    }
    let response = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/orders/query")
            .set_json(serde_json::json!({"snapshot":frozen["snapshot"]}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 204);
    assert_eq!(
        test::call_service(&app, send(frozen_query)).await.status(),
        409
    );
    for route in ["/orders/read", "/history/query", "/capabilities"] {
        assert_eq!(
            test::call_service(
                &app,
                test::TestRequest::post()
                    .uri(route)
                    .set_json(query())
                    .to_request()
            )
            .await
            .status(),
            404
        );
    }
    for field in ["page_token", "position", "consistency", "direction", "side"] {
        let mut invalid = query();
        invalid[field] = "old".into();
        assert_eq!(test::call_service(&app, send(invalid)).await.status(), 400);
    }
    let mut history = query();
    history["source"] = "history".into();
    let result: serde_json::Value = test::call_and_read_body_json(&app, send(history)).await;
    assert!(result["data"].as_array().unwrap().is_empty());
    let schema = serde_json::to_value(ApiDoc::openapi()).unwrap();
    assert!(schema["paths"]["/orders/query"]["post"].is_object());
    assert!(schema["paths"].get("/orders/read").is_none());
    for old in [
        "GiftOrdersQuery",
        "GiftHistoryQuery",
        "OrderReadQuery",
        "OrderBoundary",
        "OrderPageResponse",
        "ReadConsistency",
    ] {
        assert!(schema["components"]["schemas"].get(old).is_none(), "{old}");
    }
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
