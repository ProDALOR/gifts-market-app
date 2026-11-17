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
struct TestMarket {
    data: MarketDataHandle,
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
        let jobs = vec![
            tokio::spawn(data.task.run(stop.child_token())),
            tokio::spawn(replica.replication.run(hub, stop.child_token())),
        ];
        Self {
            data: data.handle,
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
    let gateway = MarketApi::new(runtime.data.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        while gateway.ready().await.is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    (gateway, runtime, peer, stop)
}
fn query() -> serde_json::Value {
    serde_json::json!({"scope":{"item_type_id":"tg.gift.123","currency_id":"GRAM","modes":["ask"]},"limit":25})
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
    // REST and WebSocket requests use the same compiler, including scope defaults and limits.
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
            .set_json(serde_json::json!({"scope":{"item_type_id":"tg.gift.123","currency_id":"GRAM"}}))
            .to_request()).await).await;
    assert_eq!(defaults["data"][0]["mode"], "ask");
    let directory: serde_json::Value = test::read_body_json(
        test::call_service(&app, test::TestRequest::get().uri("/books").to_request()).await,
    )
    .await;
    assert_eq!(directory["data"][0]["currency_id"], "GRAM");
    assert!(directory["next_after"].is_null());
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
                            value["page"]["data"][0]["attributes"]["number"]["value"]["value"],
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
    api_stop.cancel();
    api_task.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
#[actix_web::test]
async fn public_contract_contains_no_legacy_money_item_or_trade_models() {
    let value = ApiDoc::openapi();
    let json = value.to_json().unwrap();
    assert!(json.contains("BookKey"));
    assert!(!json.contains("Money"));
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
    let selection = "item=tg.gift.123&currency=GRAM&mode=ask&sort=price_desc&max=2";
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
    let server = ApiServer::bind(config, runtime.data.clone()).unwrap();
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    assert_eq!(http(address, "GET", "/healthz", "", "").await.0, 200);
    for (method, path, body, headers, status, code) in [
        ("GET", "/books?limit=bad", "", "", 400, "invalid_query"),
        ("GET", "/catalog/not_a_kind", "", "", 400, "invalid_query"),
        ("GET", "/books?after=bad", "", "", 400, "invalid_query"),
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
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(MarketApi::new(service.handle)))
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
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}
