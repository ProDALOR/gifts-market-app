use super::{
    operations::{self, End},
    queue::{self, Scope},
    registry::{Key, OperationId, Registry},
};
use crate::{MarketApi, config::WebSocketLimits, error::ApiError, models::Command};
use actix_ws::{AggregatedMessage, CloseCode, CloseReason};
use futures_util::{
    FutureExt, Stream, StreamExt, future::LocalBoxFuture, stream::FuturesUnordered,
};
use std::{panic::AssertUnwindSafe, time::Instant};
use tokio_util::sync::CancellationToken;
struct Settings {
    limits: WebSocketLimits,
    stop: CancellationToken,
    status: Option<tokio::sync::watch::Receiver<market_gifts::SyncStatus>>,
}
type Start = dyn Fn(
    Key,
    operations::Operation,
    queue::Sender,
    CancellationToken,
    WebSocketLimits,
) -> LocalBoxFuture<'static, End>;
type Job = LocalBoxFuture<'static, (Key, End)>;
type Write = LocalBoxFuture<'static, (queue::Frame, bool)>;

pub(super) async fn run(
    session: actix_ws::Session,
    messages: impl Stream<Item = Result<AggregatedMessage, actix_ws::ProtocolError>> + Unpin,
    market: MarketApi,
    limits: WebSocketLimits,
    stop: CancellationToken,
) {
    let status = Some(market.status_updates());
    drive(
        session,
        messages,
        Settings {
            limits,
            stop,
            status,
        },
        move |key, query, tx, cancel, limits| {
            operations::run(market.clone(), key, query, tx, cancel, limits).boxed_local()
        },
    )
    .await;
}
async fn drive(
    session: actix_ws::Session,
    messages: impl Stream<Item = Result<AggregatedMessage, actix_ws::ProtocolError>> + Unpin,
    settings: Settings,
    start: impl Fn(
        Key,
        operations::Operation,
        queue::Sender,
        CancellationToken,
        WebSocketLimits,
    ) -> LocalBoxFuture<'static, End>
    + 'static,
) {
    drive_with_writer(session, messages, settings, start, write_frame).await;
}
async fn drive_with_writer(
    session: actix_ws::Session,
    mut messages: impl Stream<Item = Result<AggregatedMessage, actix_ws::ProtocolError>> + Unpin,
    settings: Settings,
    start: impl Fn(
        Key,
        operations::Operation,
        queue::Sender,
        CancellationToken,
        WebSocketLimits,
    ) -> LocalBoxFuture<'static, End>
    + 'static,
    writer: impl Fn(actix_ws::Session, queue::Frame, WebSocketLimits) -> Write,
) {
    let Settings {
        limits,
        stop,
        mut status,
    } = settings;
    let mut jobs = FuturesUnordered::<Job>::new();
    let mut registry = Registry::new(limits.subscriptions, limits.requests);
    let (tx, mut rx) = queue::channel(&limits);
    if let Some(status) = &mut status {
        let value = status.borrow_and_update().clone();
        if tx.status(value).is_err() {
            return;
        }
    }
    let mut write: Option<Write> = None;
    let mut controls = FuturesUnordered::<LocalBoxFuture<'static, bool>>::new();
    let mut heartbeat = tokio::time::interval(limits.heartbeat);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut seen = Instant::now();
    let close = loop {
        tokio::select! {
            _=stop.cancelled()=>break CloseCode::Away,
            result=async {status.as_mut().expect("guarded status").changed().await},if status.is_some()=>{
                if result.is_err(){break CloseCode::Away;}
                let value=status.as_mut().expect("status present").borrow_and_update().clone();
                if tx.status(value).is_err(){break CloseCode::Again;}
            }
            Some((key,end))=jobs.next(),if !jobs.is_empty()=>{
                if registry.current(&key) {
                    match end {
                        End::TerminalQueued=>{},
                        End::Cancelled=>registry.finish(&key),
                        End::SlowConsumer=>break CloseCode::Again,
                        End::Failed=>break CloseCode::Error,
                    }
                }
            }
            result=async {write.as_mut().expect("guarded writer").await},if write.is_some()=>{
                write=None;
                let (frame,ok)=result;
                if !ok {break CloseCode::Again;}
                if frame.terminal && let Scope::Operation(key)=&frame.scope {registry.finish(key);}
            }
            Some(ok)=controls.next(),if !controls.is_empty()=>{if !ok {break CloseCode::Again;}}
            frame=rx.recv(),if write.is_none()=>{
                let Some(frame)=frame else {break CloseCode::Error;};
                if !frame.scope.current(&registry) {continue;}
                write=Some(writer(session.clone(),frame,limits.clone()));
            }
            _=heartbeat.tick()=>{
                if seen.elapsed()>limits.idle_timeout {break CloseCode::Away;}
                if controls.len()>=4 {break CloseCode::Again;}
                controls.push(control(session.clone(),None,limits.write_timeout));
            }
            message=messages.next()=>match message {
                Some(Ok(AggregatedMessage::Ping(value)))=>{
                    seen=Instant::now();
                    if controls.len()>=4 {break CloseCode::Again;}
                    controls.push(control(session.clone(),Some(value),limits.write_timeout));
                }
                Some(Ok(AggregatedMessage::Pong(_)))=>seen=Instant::now(),
                Some(Ok(AggregatedMessage::Text(text)))=>{
                    seen=Instant::now();
                    let command=match serde_json::from_str::<Command>(&text) {
                        Ok(command)=>command,
                        Err(_)=>{if tx.error(Scope::Global,ApiError::invalid("invalid API message")).is_err(){break CloseCode::Again;}continue;}
                    };
                    if schedule(command,&mut registry,&mut jobs,&tx,&limits,&stop,&start).is_err(){break CloseCode::Again;}
                }
                Some(Ok(AggregatedMessage::Close(_)))|None=>break CloseCode::Normal,
                _=>break CloseCode::Protocol,
            }
        }
    };
    stop.cancel();
    drop(jobs);
    drop(write);
    drop(controls);
    drop(registry);
    drop(rx);
    let reason = CloseReason {
        code: close,
        description: match close {
            CloseCode::Again => Some("Connection cannot keep up; reconnect to resubscribe".into()),
            CloseCode::Error => Some("API operation failed".into()),
            _ => None,
        },
    };
    let _ = tokio::time::timeout(limits.write_timeout, session.close(Some(reason))).await;
}
fn write_frame(
    mut session: actix_ws::Session,
    frame: queue::Frame,
    limits: WebSocketLimits,
) -> Write {
    async move {
        let text = frame.payload.text(frame.scope.id());
        let ok = matches!(
            tokio::time::timeout(
                limits.write_timeout,
                write_text(&mut session, text, limits.frame_bytes)
            )
            .await,
            Ok(Ok(()))
        );
        (frame, ok)
    }
    .boxed_local()
}
fn control(
    mut session: actix_ws::Session,
    pong: Option<actix_web::web::Bytes>,
    deadline: std::time::Duration,
) -> LocalBoxFuture<'static, bool> {
    async move {
        matches!(
            tokio::time::timeout(deadline, async {
                match pong {
                    Some(value) => session.pong(&value).await,
                    None => session.ping(b"market").await,
                }
            })
            .await,
            Ok(Ok(()))
        )
    }
    .boxed_local()
}
fn schedule(
    command: Command,
    registry: &mut Registry,
    jobs: &mut FuturesUnordered<Job>,
    tx: &queue::Sender,
    limits: &WebSocketLimits,
    stop: &CancellationToken,
    start: &Start,
) -> Result<(), queue::QueueError> {
    let (id, query) = match command {
        Command::WatchOrders { id, params } => (id, Some(operations::Operation::Orders(params))),
        Command::WatchCatalog { id, collection_ids } => {
            (id, Some(operations::Operation::Catalog(collection_ids)))
        }
        Command::Cancel { id } => (id, None),
        Command::Request(request) => (
            request.id().to_owned(),
            Some(operations::Operation::Request(request)),
        ),
    };
    let id = match OperationId::parse(id) {
        Ok(id) => id,
        Err(error) => return tx.error(Scope::Global, error),
    };
    let Some(query) = query else {
        registry.remove(&id);
        return Ok(());
    };
    if let Some(key) = registry.key(&id) {
        return tx.reject(
            Scope::Operation(key),
            ApiError::invalid("duplicate operation ID"),
        );
    }
    let reserved = if jobs.len()
        >= limits
            .subscriptions
            .saturating_add(limits.requests)
            .saturating_mul(2)
    {
        Err(ApiError::capacity("operation setup capacity reached"))
    } else if matches!(&query, operations::Operation::Request(_)) {
        registry.request(id.clone(), stop)
    } else {
        registry.reserve(id.clone(), stop)
    };
    let (key, cancel) = match reserved {
        Ok(value) => value,
        Err(error) => return tx.error(Scope::Absent(id), error),
    };
    let work = start(key.clone(), query, tx.clone(), cancel, limits.clone());
    jobs.push(
        async move {
            let result = AssertUnwindSafe(work).catch_unwind().await;
            (
                key,
                result.unwrap_or_else(|_| {
                    tracing::warn!("WebSocket operation task panicked");
                    End::Failed
                }),
            )
        }
        .boxed_local(),
    );
    Ok(())
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
