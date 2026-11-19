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
    block_writes: bool,
    write_started: Arc<Notify>,
    allow_write: Arc<Notify>,
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
        requests: 1,
        ..Default::default()
    };
    let writer_scenario = scenario.clone();
    actix_web::rt::spawn(drive_with_writer(
        session,
        messages.aggregate_continuations(),
        Settings {
            limits,
            stop: scenario.stop.child_token(),
            status: None,
        },
        move |key, query, tx, cancel, _| {
            let scenario = scenario.clone();
            async move {
            let query = match query {
                operations::Operation::Orders(params) => params.query,
                operations::Operation::Request(_) => {
                    scenario.started.notify_one();cancel.cancelled().await;scenario.cancelled.notify_one();return End::Cancelled;
                },
                _ => return End::Failed,
            };
            if query.limit==1 {
                scenario.started.notify_one();cancel.cancelled().await;scenario.cancelled.notify_one();return End::Cancelled;
            }
            if query.limit==4 {return End::SlowConsumer;}
            let label=if query.limit==3 {"😀".repeat(20_000)} else {"ready".into()};
            let payload = queue::Payload::Orders(serde_json::json!({"type":"orders.snapshot","id":key.id.as_str(),"sequence":0,"data":[],"range":{"start":null,"end":null},"label":label}).to_string().into());
            if tx.send(Scope::Operation(key),payload,false,&cancel).await.is_err() {return End::SlowConsumer;}
            cancel.cancelled().await;scenario.cancelled.notify_one();End::Cancelled
        }.boxed_local()
        },
        move |session, frame, limits| {
            let scenario = writer_scenario.clone();
            async move {
                if scenario.block_writes {
                    scenario.write_started.notify_one();
                    scenario.allow_write.notified().await;
                }
                write_frame(session, frame, limits).await
            }
            .boxed_local()
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
    server_with_writer(false).await
}
async fn server_with_writer(
    block_writes: bool,
) -> (
    Scenario,
    Socket,
    actix_web::dev::ServerHandle,
    actix_web::rt::task::JoinHandle<std::io::Result<()>>,
) {
    let scenario = Scenario {
        stop: CancellationToken::new(),
        started: Arc::default(),
        cancelled: Arc::default(),
        block_writes,
        write_started: Arc::default(),
        allow_write: Arc::default(),
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
            serde_json::json!({"type":"orders.watch","id":"offers","params":{"query":{"filters":{"collection_id":"gift","currency_id":"GRAM"},"limit":limit}}})
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
async fn pending_setup_does_not_block_ping_cancel_replacement_or_shutdown() {
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
            "{\"type\":\"cancel\",\"id\":\"offers\"}".into(),
        ))
        .await
        .unwrap();
    subscribe(&mut socket, 2).await;
    let Message::Text(text) = next(&mut socket).await else {
        panic!("missing replacement page")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["label"],
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
    assert_eq!(page["label"].as_str().unwrap().chars().count(), 20_000);
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

#[actix_web::test]
async fn request_capacity_and_cancel_do_not_interrupt_a_healthy_subscription() {
    let (scenario, mut socket, server, job) = server().await;
    subscribe(&mut socket, 2).await;
    assert!(matches!(next(&mut socket).await, Message::Text(_)));
    let request = |id: &str| {
        Message::Text(serde_json::json!({"type":"request","id":id,"method":"valuation.evaluate","params":{"gift":"Gift-123","currency_id":"GRAM"}}).to_string().into())
    };
    socket.send(request("first")).await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        scenario.started.notified(),
    )
    .await
    .unwrap();
    socket.send(request("second")).await.unwrap();
    let Message::Text(text) = next(&mut socket).await else {
        panic!("error expected")
    };
    let error: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(error["id"], "second");
    assert_eq!(error["error"]["code"], "capacity_exceeded");
    socket
        .send(Message::Text(
            "{\"type\":\"cancel\",\"id\":\"first\"}".into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        scenario.cancelled.notified(),
    )
    .await
    .unwrap();
    socket
        .send(Message::Ping(b"still-live".to_vec().into()))
        .await
        .unwrap();
    assert!(matches!(next(&mut socket).await, Message::Pong(_)));
    socket.send(request("third")).await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        scenario.started.notified(),
    )
    .await
    .unwrap();
    scenario.stop.cancel();
    assert!(matches!(next(&mut socket).await, Message::Close(_)));
    server.stop(true).await;
    job.await.unwrap().unwrap();
}

#[actix_web::test]
async fn malformed_commands_fail_validation_without_interrupting_existing_work() {
    let (scenario, mut socket, server, job) = server().await;
    subscribe(&mut socket, 2).await;
    assert!(matches!(next(&mut socket).await, Message::Text(_)));
    socket
        .send(Message::Text(
            serde_json::json!({"type":"request","id":"pending","method":"valuation.evaluate","params":{"gift":"Gift-123","currency_id":"GRAM"}})
                .to_string().into(),
        ))
        .await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        scenario.started.notified(),
    )
    .await
    .unwrap();
    for invalid in [
        serde_json::json!({"type":"request","id":"bad id","method":"valuation.evaluate","params":{"gift":"Gift-123","currency_id":"GRAM"}}),
        serde_json::json!({"type":"request","id":"x","method":"unknown","params":{}}),
        serde_json::json!({"type":"request","id":"x","method":"valuation.evaluate","params":{"gift":"Gift-123","currency_id":"GRAM","unexpected":true}}),
        serde_json::json!({"type":"cancel","id":"x","unexpected":true}),
    ] {
        socket
            .send(Message::Text(invalid.to_string().into()))
            .await
            .unwrap();
        let Message::Text(text) = next(&mut socket).await else {
            panic!("error expected")
        };
        let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(frame["type"], "command_error");
        assert_eq!(frame["id"], "");
        assert_eq!(frame["error"]["code"], "invalid_query");
    }
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            scenario.cancelled.notified()
        )
        .await
        .is_err()
    );
    socket
        .send(Message::Text(
            serde_json::json!({"type":"cancel","id":"pending"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        scenario.cancelled.notified(),
    )
    .await
    .unwrap();
    socket
        .send(Message::Text(
            "{\"type\":\"cancel\",\"id\":\"unknown\"}".into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Ping(b"healthy".to_vec().into()))
        .await
        .unwrap();
    assert!(matches!(next(&mut socket).await, Message::Pong(_)));
    scenario.stop.cancel();
    assert!(matches!(next(&mut socket).await, Message::Close(_)));
    server.stop(true).await;
    job.await.unwrap().unwrap();
}

#[actix_web::test]
async fn cancellation_and_pings_progress_while_a_data_write_is_blocked() {
    let (scenario, mut socket, server, job) = server_with_writer(true).await;
    subscribe(&mut socket, 2).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        scenario.write_started.notified(),
    )
    .await
    .unwrap();
    socket
        .send(Message::Text(
            "{\"type\":\"cancel\",\"id\":\"offers\"}".into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_millis(200),
        scenario.cancelled.notified(),
    )
    .await
    .expect("Cancellation must not await the write deadline");
    socket
        .send(Message::Ping(b"responsive".to_vec().into()))
        .await
        .unwrap();
    assert!(matches!(next(&mut socket).await, Message::Pong(_)));
    scenario.allow_write.notify_one();
    assert!(matches!(next(&mut socket).await, Message::Text(_)));
    scenario.stop.cancel();
    server.stop(true).await;
    job.await.unwrap().unwrap();
}
#[actix_web::test]
async fn a_duplicate_request_is_a_command_rejection_and_leaves_the_original_running() {
    let (scenario, mut socket, server, job) = server().await;
    let request=Message::Text(serde_json::json!({"type":"request","id":"r","method":"valuation.evaluate","params":{"gift":"Gift-123","currency_id":"GRAM"}}).to_string().into());
    socket.send(request.clone()).await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        scenario.started.notified(),
    )
    .await
    .unwrap();
    socket.send(request).await.unwrap();
    let Message::Text(text) = next(&mut socket).await else {
        panic!("command rejection expected")
    };
    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(frame["type"], "command_error");
    assert_eq!(frame["id"], "r");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            scenario.cancelled.notified()
        )
        .await
        .is_err()
    );
    socket
        .send(Message::Text("{\"type\":\"cancel\",\"id\":\"r\"}".into()))
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        scenario.cancelled.notified(),
    )
    .await
    .unwrap();
    scenario.stop.cancel();
    server.stop(true).await;
    job.await.unwrap().unwrap();
}
