use crate::{
    api::{Command, ServerFrame},
    error::ApiError,
    gateway::{Gateway, Update},
};
use actix_web::{HttpRequest, HttpResponse, web};
use actix_ws::AggregatedMessage;
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn json(frame: ServerFrame) -> String {
    serde_json::to_string(&frame).expect("public frame serializes")
}
async fn forward(
    id: String,
    _feed: std::sync::Arc<crate::gateway::Live>,
    mut updates: tokio::sync::watch::Receiver<Update>,
    tx: mpsc::Sender<String>,
    cancel: CancellationToken,
) {
    loop {
        let update = updates.borrow_and_update().clone();
        let terminal = matches!(update, Update::Error(_));
        let frame = match update {
            Update::Connecting => ServerFrame::Status {
                id: id.clone(),
                state: "connecting".into(),
            },
            Update::Page(page) => ServerFrame::Page {
                id: id.clone(),
                page,
            },
            Update::Error(error) => ServerFrame::Error {
                id: id.clone(),
                error,
            },
        };
        let value = json(frame);
        if value.len() > 4 * 1024 * 1024 {
            break;
        }
        let delivered = tokio::select! {
            _ = cancel.cancelled() => false,
            result = tokio::time::timeout(Duration::from_secs(5), tx.send(value)) => matches!(result, Ok(Ok(()))),
        };
        if !delivered || terminal {
            break;
        }
        let closed = tokio::select! {
            _ = cancel.cancelled() => true,
            result = updates.changed() => result.is_err(),
        };
        if closed {
            break;
        }
    }
    cancel.cancel();
}

pub async fn connect(
    req: HttpRequest,
    payload: web::Payload,
    state: web::Data<Gateway>,
    origins: web::Data<Vec<String>>,
) -> actix_web::Result<HttpResponse> {
    if let Some(origin) = req.headers().get("origin").and_then(|v| v.to_str().ok()) {
        let host = req.connection_info().host().to_owned();
        if origin != format!("http://{host}")
            && origin != format!("https://{host}")
            && !origins.iter().any(|v| v == origin)
        {
            return Ok(HttpResponse::Forbidden().finish());
        }
    }
    let (response, mut session, messages) = actix_ws::handle(&req, payload)?;
    let mut messages = messages
        .max_frame_size(64 * 1024)
        .aggregate_continuations()
        .max_continuation_size(64 * 1024);
    actix_web::rt::spawn(async move {
        let stop = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel::<String>(8);
        let mut subscriptions: HashMap<String, CancellationToken> = HashMap::new();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
        let mut seen = Instant::now();
        loop {
            tokio::select! {
                value = rx.recv() => match value {
                    Some(value) => if !matches!(tokio::time::timeout(Duration::from_secs(5), session.text(value)).await, Ok(Ok(()))) { break; },
                    None => break,
                },
                _ = heartbeat.tick() => {
                    if seen.elapsed() > Duration::from_secs(35) { break; }
                    if session.ping(b"market").await.is_err() { break; }
                },
                value = messages.next() => match value {
                    Some(Ok(AggregatedMessage::Ping(value))) => {
                        seen = Instant::now();
                        if session.pong(&value).await.is_err() { break; }
                    },
                    Some(Ok(AggregatedMessage::Pong(_))) => seen = Instant::now(),
                    Some(Ok(AggregatedMessage::Text(text))) => {
                        seen = Instant::now();
                        subscriptions.retain(|_, value| !value.is_cancelled());
                        let mut id = String::new();
                        let result = match serde_json::from_str::<Command>(&text) {
                            Ok(Command::Unsubscribe { id }) => {
                                if let Some(cancel) = subscriptions.remove(&id) { cancel.cancel(); }
                                Ok(())
                            },
                            Ok(Command::Subscribe { id: subscription, query }) => {
                                id = subscription.clone();
                                if subscription.is_empty() || subscription.len() > 64
                                    || !subscription.bytes().all(|v| v.is_ascii_alphanumeric() || b"_.-".contains(&v)) {
                                    Err(ApiError::invalid("invalid subscription ID"))
                                } else if subscriptions.contains_key(&subscription) || subscriptions.len() >= 16 {
                                    Err(ApiError::invalid("duplicate subscription or subscription limit reached"))
                                } else {
                                    state.subscribe(query).map(|(feed, updates)| {
                                        let cancel = stop.child_token();
                                        subscriptions.insert(subscription.clone(), cancel.clone());
                                        actix_web::rt::spawn(forward(subscription, feed, updates, tx.clone(), cancel));
                                    })
                                }
                            },
                            Err(_) => Err(ApiError::invalid("invalid subscription message")),
                        };
                        if let Err(error) = result
                            && session.text(json(ServerFrame::Error { id, error })).await.is_err() { break; }
                    },
                    _ => break,
                }
            }
        }
        stop.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(5), session.close(None)).await;
    });
    Ok(response)
}
