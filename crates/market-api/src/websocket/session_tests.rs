use super::*;
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
use futures_util::SinkExt;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_tungstenite::{connect_async, tungstenite::Message};
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
#[derive(Clone)]
struct Scenario {
    stop: CancellationToken,
    started: Arc<Notify>,
    cancelled: Arc<Notify>,
}
async fn connect(
    request: HttpRequest,
    payload: web::Payload,
    state: web::Data<Scenario>,
) -> actix_web::Result<HttpResponse> {
    let (response, session, messages) = actix_ws::handle(&request, payload)?;
    let scenario = state.get_ref().clone();
    let limits = WebSocketLimits {
        frame_bytes: 1023,
        ..Default::default()
    };
    actix_web::rt::spawn(drive(
        session,
        messages.aggregate_continuations(),
        limits,
        scenario.stop.child_token(),
        move |key, query, tx, cancel, _| {
            let scenario = scenario.clone();
            async move {
            let subscription::Query::Orders(query) = query else { return End::Failed; };
            if query.limit==1 {
                scenario.started.notify_one();cancel.cancelled().await;scenario.cancelled.notify_one();return End::Cancelled;
            }
            if query.limit==4 {return End::SlowConsumer;}
            let label=if query.limit==3 {"😀".repeat(20_000)} else {"ready".into()};
            let page=serde_json::json!({"data":[],"cursor":label,"next_page_token":"","sync":{"ready":true,"state":"live","updated_at":null}}).to_string();
            if tx.send(Scope::Subscription(key),queue::Payload::test_page(Arc::from(page)),false,&cancel).await.is_err() {return End::SlowConsumer;}
            cancel.cancelled().await;End::Cancelled
        }.boxed_local()
        },
    ));
    Ok(response)
}
async fn server() -> (
    Scenario,
    Socket,
    actix_web::dev::ServerHandle,
    actix_web::rt::task::JoinHandle<std::io::Result<()>>,
) {
    let scenario = Scenario {
        stop: CancellationToken::new(),
        started: Arc::default(),
        cancelled: Arc::default(),
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = web::Data::new(scenario.clone());
    let server = HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .route("/ws", web::get().to(connect))
    })
    .workers(1)
    .disable_signals()
    .shutdown_timeout(1)
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    let job = actix_web::rt::spawn(server);
    let (socket, _) = connect_async(format!("ws://{address}/ws")).await.unwrap();
    (scenario, socket, handle, job)
}
async fn subscribe(socket: &mut Socket, limit: u32) {
    socket
        .send(Message::Text(
            serde_json::json!({"type":"subscribe","id":"offers","query":{"filters":{"collection_id":"gift","currency_id":"GRAM"},"limit":limit}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}
async fn next(socket: &mut Socket) -> Message {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Ping(value) => socket.send(Message::Pong(value)).await.unwrap(),
                message => return message,
            }
        }
    })
    .await
    .unwrap()
}
#[actix_web::test]
async fn pending_setup_does_not_block_ping_unsubscribe_replacement_or_shutdown() {
    let (scenario, mut socket, server, job) = server().await;
    subscribe(&mut socket, 1).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        scenario.started.notified(),
    )
    .await
    .unwrap();
    socket
        .send(Message::Ping(b"control".to_vec().into()))
        .await
        .unwrap();
    assert!(matches!(next(&mut socket).await,Message::Pong(value) if value.as_ref()==b"control"));
    socket
        .send(Message::Text(
            "{\"type\":\"unsubscribe\",\"id\":\"offers\"}".into(),
        ))
        .await
        .unwrap();
    subscribe(&mut socket, 2).await;
    let Message::Text(text) = next(&mut socket).await else {
        panic!("missing replacement page")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["page"]["cursor"],
        "ready"
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        scenario.cancelled.notified(),
    )
    .await
    .unwrap();
    scenario.stop.cancel();
    assert!(
        matches!(next(&mut socket).await,Message::Close(Some(frame)) if u16::from(frame.code)==1001)
    );
    server.stop(true).await;
    job.await.unwrap().unwrap();
}
#[actix_web::test]
async fn fragmented_large_unicode_pages_preserve_one_json_message() {
    let (scenario, mut socket, server, job) = server().await;
    subscribe(&mut socket, 3).await;
    let Message::Text(text) = next(&mut socket).await else {
        panic!("missing page")
    };
    let page: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        page["page"]["cursor"].as_str().unwrap().chars().count(),
        20_000
    );
    socket.close(None).await.unwrap();
    scenario.stop.cancel();
    server.stop(true).await;
    job.await.unwrap().unwrap();
}
#[actix_web::test]
async fn slow_consumers_get_a_reconnect_close_instead_of_a_silent_subscription_stop() {
    let (scenario, mut socket, server, job) = server().await;
    subscribe(&mut socket, 4).await;
    assert!(
        matches!(next(&mut socket).await,Message::Close(Some(frame)) if u16::from(frame.code)==1013)
    );
    scenario.stop.cancel();
    server.stop(true).await;
    job.await.unwrap().unwrap();
}
