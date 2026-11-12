use crate::{
    error::{ApiError, ErrorCode, Result},
    query::{ApiState, BookQuery},
    status::ApiCursor,
};
use actix_web::{HttpRequest, HttpResponse, web};
use actix_ws::AggregatedMessage;
use futures_util::StreamExt;
use serde::Deserialize;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const MAX_INPUT: usize = 64 * 1024;
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
#[derive(Clone, Default)]
pub struct Origins(pub Vec<String>);
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Subscribe {
        subscription_id: String,
        #[serde(flatten)]
        topic: Box<Topic>,
    },
    Unsubscribe {
        subscription_id: String,
    },
}
#[derive(Deserialize)]
#[serde(tag = "topic", rename_all = "snake_case", deny_unknown_fields)]
enum Topic {
    Orderbook { query: BookQuery },
    Changes { after: Option<ApiCursor> },
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"_-.:".contains(&v))
}
fn origin_allowed(req: &HttpRequest, origins: &Origins) -> bool {
    let Some(origin) = req.headers().get("origin") else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let host = req.connection_info().host().to_owned();
    origin == format!("http://{host}")
        || origin == format!("https://{host}")
        || origins.0.iter().any(|v| v == origin)
}
#[utoipa::path(get,path="/ws",responses((status=101,description="Read-only WebSocket subscriptions: orderbook or changes"),(status=429,body=crate::error::ErrorResponse)))]
pub async fn connect(
    req: HttpRequest,
    payload: web::Payload,
    state: web::Data<ApiState>,
    origins: Option<web::Data<Origins>>,
) -> Result<HttpResponse> {
    if !req.query_string().is_empty() {
        return Err(ApiError::invalid_query(
            "WebSocket accepts subscriptions as JSON messages",
        ));
    }
    if !origin_allowed(
        &req,
        &origins.map(|v| v.get_ref().clone()).unwrap_or_default(),
    ) {
        return Ok(HttpResponse::Forbidden().json(crate::error::ErrorResponse {
            error: ApiError::invalid_query("WebSocket origin is not allowed"),
        }));
    }
    let permit = state.stream_slot()?;
    let (response, mut session, messages) =
        actix_ws::handle(&req, payload).map_err(ApiError::invalid_query)?;
    let mut messages = messages
        .max_frame_size(MAX_INPUT)
        .aggregate_continuations()
        .max_continuation_size(MAX_INPUT);
    let state = state.get_ref().clone();
    actix_web::rt::spawn(async move {
        let _permit = permit;
        let stop = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel::<String>(8);
        let mut subscriptions: HashMap<String, CancellationToken> = HashMap::new();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
        let mut last_seen = Instant::now();
        loop {
            tokio::select! {
                _=stop.cancelled()=>break,
                message=rx.recv()=>match message {
                    Some(message)=>if !matches!(tokio::time::timeout(SEND_TIMEOUT,session.text(message)).await,Ok(Ok(()))) {break;},
                    None=>break,
                },
                _=heartbeat.tick()=>{
                    if last_seen.elapsed()>Duration::from_secs(30) {break;}
                    if !matches!(tokio::time::timeout(SEND_TIMEOUT,session.ping(b"market")).await,Ok(Ok(()))) {break;}
                },
                message=messages.next()=>{
                    match message {
                        Some(Ok(AggregatedMessage::Ping(value)))=>{last_seen=Instant::now();if !matches!(tokio::time::timeout(SEND_TIMEOUT,session.pong(&value)).await,Ok(Ok(()))) {break;}},
                        Some(Ok(AggregatedMessage::Pong(_)))=>last_seen=Instant::now(),
                        Some(Ok(AggregatedMessage::Text(text)))=>{
                            last_seen=Instant::now();
                            subscriptions.retain(|_,v|!v.is_cancelled());
                            let command=serde_json::from_str::<Command>(&text).map_err(ApiError::invalid_query);
                            let error_id=command.as_ref().ok().map(|command| match command {Command::Subscribe {subscription_id,..}|Command::Unsubscribe {subscription_id}=>subscription_id.clone()});
                            let result=match command {
                                Ok(Command::Unsubscribe {subscription_id})=>{
                                    if let Some(token)=subscriptions.remove(&subscription_id) {token.cancel();}
                                    send(&tx,&stop,serde_json::json!({"type":"unsubscribed","subscription_id":subscription_id}).to_string()).await; Ok(())
                                },
                                Ok(Command::Subscribe {subscription_id,topic})=>{
                                    if !valid_id(&subscription_id) {Err(ApiError::field("subscription_id","use 1..64 ASCII letters, digits, _, -, . or :"))}
                                    else if subscriptions.contains_key(&subscription_id) {Err(ApiError::field("subscription_id","already subscribed"))}
                                    else if subscriptions.len()>=16 {Err(ApiError::new(ErrorCode::CapacityExceeded,"at most 16 subscriptions per socket"))}
                                    else {
                                        let token=stop.child_token();
                                        match subscribe(state.clone(),subscription_id.clone(),*topic,token.clone(),tx.clone(),stop.clone()) {
                                            Ok(())=>{subscriptions.insert(subscription_id,token);Ok(())},Err(e)=>Err(e),
                                        }
                                    }
                                },
                                Err(e)=>Err(e),
                            };
                            if let Err(error)=result {send(&tx,&stop,serde_json::json!({"type":"error","subscription_id":error_id,"error":error}).to_string()).await;}
                        },
                        Some(Ok(AggregatedMessage::Binary(_)))=>{send(&tx,&stop,serde_json::json!({"type":"error","error":ApiError::invalid_query("JSON text messages required")}).to_string()).await;},
                        _=>break,
                    }
                }
            }
        }
        stop.cancel();
        let _ = tokio::time::timeout(SEND_TIMEOUT, session.close(None)).await;
    });
    Ok(response)
}
async fn send(tx: &mpsc::Sender<String>, stop: &CancellationToken, text: String) -> bool {
    if text.len() > 4 * 1024 * 1024 {
        stop.cancel();
        return false;
    }
    tokio::select! {
        _=stop.cancelled()=>false,
        result=tokio::time::timeout(SEND_TIMEOUT,tx.send(text))=>{
            if matches!(result,Ok(Ok(()))) {true} else {stop.cancel();false}
        }
    }
}
fn subscribe(
    state: ApiState,
    id: String,
    topic: Topic,
    token: CancellationToken,
    tx: mpsc::Sender<String>,
    stop: CancellationToken,
) -> Result<()> {
    let permit = state.stream_slot()?;
    actix_web::rt::spawn(async move {
        let work = async {
            let _permit = permit;
            match topic {
                Topic::Orderbook { query } => {
                    let mut stream = state.watch_book(&query).await?;
                    let mut first = true;
                    while let Some(frame) = stream.message().await.map_err(ApiError::from)? {
                        let response = crate::query::book_response(frame)?;
                        let kind = if first { "snapshot" } else { "replace" };
                        first = false;
                        let text=serde_json::json!({"type":kind,"subscription_id":id,"response":response}).to_string();
                        if !send(&tx, &stop, text).await {
                            break;
                        }
                    }
                }
                Topic::Changes { after } => {
                    let mut stream = state
                        .sync(hub_protocol::wire::SyncRequest {
                            after: after.map(|c| hub_protocol::wire::Cursor {
                                generation: c.generation,
                                revision: c.revision,
                            }),
                        })
                        .await
                        .map_err(ApiError::from)?;
                    while let Some(frame) = stream.message().await.map_err(ApiError::from)? {
                        let text=serde_json::json!({"type":"sync","subscription_id":id,"response":crate::query::sync_response(frame)?}).to_string();
                        if !send(&tx, &stop, text).await {
                            break;
                        }
                    }
                }
            }
            Ok::<_, ApiError>(())
        };
        tokio::select! {_=token.cancelled()=>{},r=work=>{if let Err(error)=r {send_error(&tx,&stop,&id,error).await;}}}
        token.cancel();
    });
    Ok(())
}

async fn send_error(
    tx: &mpsc::Sender<String>,
    stop: &CancellationToken,
    id: &str,
    error: ApiError,
) {
    let kind = if matches!(error.code, ErrorCode::ResyncRequired) {
        "resync_required"
    } else {
        "error"
    };
    send(
        tx,
        stop,
        serde_json::json!({"type":kind,"subscription_id":id,"error":error}).to_string(),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subscriptions_have_one_strict_shape_and_string_revisions() {
        assert!(serde_json::from_str::<Command>(r#"{"type":"subscribe","subscription_id":"book","topic":"orderbook","query":{"item_type_id":"type","currency_id":"TON"}}"#).is_ok());
        assert!(serde_json::from_str::<Command>(r#"{"type":"subscribe","subscription_id":"raw","topic":"changes","after":{"generation":"g","revision":"9007199254740993"}}"#).is_ok());
        for raw in [
            r#"{"type":"subscribe","subscription_id":"raw","topic":"changes","after":{"generation":"g","revision":1}}"#,
            r#"{"type":"subscribe","subscription_id":"book","topic":"orderbook","query":{"item_type_id":"type","currency_id":"TON","unknown":true}}"#,
            r#"{"type":"subscribe","subscription_id":"book","topic":"unknown","query":{}}"#,
            r#"{"type":"subscribe","subscription_id":"raw","topic":"changes","unknown":true}"#,
        ] {
            assert!(serde_json::from_str::<Command>(raw).is_err(), "{raw}");
        }
    }
}
