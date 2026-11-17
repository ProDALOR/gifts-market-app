use super::{
    queue::{self, Scope},
    registry::{Key, Phase, Registry, SubscriptionId},
    subscription::{self, End},
};
use crate::{MarketApi, config::WebSocketLimits, error::ApiError, models::Command};
use actix_ws::{AggregatedMessage, CloseCode, CloseReason};
use futures_util::{
    FutureExt, Stream, StreamExt, future::LocalBoxFuture, stream::FuturesUnordered,
};
use std::{panic::AssertUnwindSafe, time::Instant};
use tokio_util::sync::CancellationToken;

pub(super) async fn run(
    session: actix_ws::Session,
    messages: impl Stream<Item = Result<AggregatedMessage, actix_ws::ProtocolError>> + Unpin,
    market: MarketApi,
    limits: WebSocketLimits,
    stop: CancellationToken,
) {
    drive(
        session,
        messages,
        limits,
        stop,
        move |key, query, tx, cancel, limits| {
            subscription::run(market.clone(), key, query, tx, cancel, limits).boxed_local()
        },
    )
    .await;
}
async fn drive(
    mut session: actix_ws::Session,
    mut messages: impl Stream<Item = Result<AggregatedMessage, actix_ws::ProtocolError>> + Unpin,
    limits: WebSocketLimits,
    stop: CancellationToken,
    start: impl Fn(
        Key,
        subscription::Query,
        queue::Sender,
        CancellationToken,
        WebSocketLimits,
    ) -> LocalBoxFuture<'static, End>,
) {
    let mut jobs = FuturesUnordered::<LocalBoxFuture<'static, (Key, End)>>::new();
    let mut registry = Registry::new(limits.subscriptions);
    let (tx, mut rx) = queue::channel(&limits);
    let mut heartbeat = tokio::time::interval(limits.heartbeat);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut seen = Instant::now();
    let close = loop {
        tokio::select! {
            _=stop.cancelled()=>break CloseCode::Away,
            Some((key,end))=jobs.next(),if !jobs.is_empty()=>{
                if registry.current(&key) {
                    match end {
                        End::TerminalQueued=>registry.phase(&key,Phase::Finishing),
                        End::Cancelled=>registry.finish(&key),
                        End::SlowConsumer=>break CloseCode::Again,
                        End::Failed=>break CloseCode::Error,
                    }
                }
            }
            frame=rx.recv()=>{
                let Some(frame)=frame else {break CloseCode::Error;};
                if !frame.scope.current(&registry) {continue;}
                let text=frame.payload.text(frame.scope.id());
                if !matches!(tokio::time::timeout(limits.write_timeout,write_text(&mut session,text,limits.frame_bytes)).await,Ok(Ok(()))) {break CloseCode::Again;}
                if let Scope::Subscription(key)=&frame.scope {
                    if frame.terminal {registry.finish(key);} else if !matches!(&frame.payload,queue::Payload::Error(_)) {registry.phase(key,Phase::Active);}
                }
            }
            _=heartbeat.tick()=>{
                if seen.elapsed()>limits.idle_timeout {break CloseCode::Away;}
                if !matches!(tokio::time::timeout(limits.write_timeout,session.ping(b"market")).await,Ok(Ok(()))) {break CloseCode::Again;}
            }
            message=messages.next()=>match message {
                Some(Ok(AggregatedMessage::Ping(value)))=>{
                    seen=Instant::now();
                    if !matches!(tokio::time::timeout(limits.write_timeout,session.pong(&value)).await,Ok(Ok(()))) {break CloseCode::Again;}
                }
                Some(Ok(AggregatedMessage::Pong(_)))=>seen=Instant::now(),
                Some(Ok(AggregatedMessage::Text(text)))=>{
                    seen=Instant::now();
                    let command=match serde_json::from_str::<Command>(&text) {
                        Ok(command)=>command,
                        Err(_)=>{if tx.error(Scope::Global,ApiError::invalid("invalid subscription message")).is_err() {break CloseCode::Again;}continue;}
                    };
                    let (id,query)=match command {Command::Subscribe{id,query}=>(id,Some(subscription::Query::Orders(query))),Command::WatchCatalog{id,collection_ids}=>(id,Some(subscription::Query::Catalog(collection_ids))),Command::Unsubscribe{id}=>(id,None)};
                    let id=match SubscriptionId::parse(id) {
                        Ok(id)=>id,
                        Err(error)=>{if tx.error(Scope::Global,error).is_err() {break CloseCode::Again;}continue;}
                    };
                    let Some(query)=query else {registry.remove(&id);continue;};
                    let reserved=if jobs.len()>=limits.subscriptions.saturating_mul(2) {Err(ApiError::capacity("subscription setup capacity reached"))}
                        else {registry.reserve(id.clone(),&stop)};
                    let (key,cancel)=match reserved {
                        Ok(reserved)=>reserved,
                        Err(error)=>{
                            let scope=registry.key(&id).map(Scope::Subscription).unwrap_or(Scope::Absent(id));
                            if tx.error(scope,error).is_err() {break CloseCode::Again;}continue;
                        }
                    };
                    let work=start(key.clone(),query,tx.clone(),cancel,limits.clone());
                    jobs.push(async move {
                        let result=AssertUnwindSafe(work).catch_unwind().await;
                        (key,result.unwrap_or_else(|_| {tracing::warn!("WebSocket subscription task panicked");End::Failed}))
                    }.boxed_local());
                }
                Some(Ok(AggregatedMessage::Close(_)))|None=>break CloseCode::Normal,
                _=>break CloseCode::Protocol,
            }
        }
    };
    stop.cancel();
    drop(jobs);
    drop(registry);
    drop(rx);
    let reason = CloseReason {
        code: close,
        description: match close {
            CloseCode::Again => Some("Connection cannot keep up; reconnect to resubscribe".into()),
            CloseCode::Error => Some("Subscription service failed".into()),
            _ => None,
        },
    };
    let _ = tokio::time::timeout(limits.write_timeout, session.close(Some(reason))).await;
}

/// Fragment large messages so Actix's internal message queue cannot retain 32
/// complete multi-megabyte pages. Fragments reassemble into the same JSON message.
async fn write_text(
    session: &mut actix_ws::Session,
    text: String,
    chunk: usize,
) -> Result<(), actix_ws::Closed> {
    use actix_web::web::Bytes;
    use actix_ws::Item;
    if text.len() <= chunk {
        return session.text(text).await;
    }
    let bytes = Bytes::from(text);
    let length = bytes.len();
    let mut start = 0;
    while start < length {
        let end = (start + chunk).min(length);
        let item = if start == 0 {
            Item::FirstText(bytes.slice(start..end))
        } else if end == length {
            Item::Last(bytes.slice(start..end))
        } else {
            Item::Continue(bytes.slice(start..end))
        };
        session.continuation(item).await?;
        start = end;
    }
    Ok(())
}
#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
