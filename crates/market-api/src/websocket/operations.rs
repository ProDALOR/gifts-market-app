//! Cancellable typed requests and subscription forwarding, independent of session I/O.
use super::{
    queue::{Payload, QueueError, Scope, Sender},
    registry::Key,
};
use crate::{MarketApi, config::WebSocketLimits, error::ApiError};
use market_actors::DataUpdate;
use tokio_util::sync::CancellationToken;
pub(super) enum Operation {
    Request(crate::models::ApiRequest),
    Window(Box<market_gifts::GiftOrderWindowQuery>),
    Catalog(Vec<market_gifts::CollectionId>),
}
#[derive(Debug)]
pub(super) enum End {
    TerminalQueued,
    Cancelled,
    SlowConsumer,
    Failed,
}
fn end(result: Result<(), QueueError>) -> End {
    match result {
        Ok(()) => End::TerminalQueued,
        Err(QueueError::Cancelled) => End::Cancelled,
        Err(QueueError::SlowConsumer) => End::SlowConsumer,
    }
}
pub(super) async fn run(
    market: MarketApi,
    key: Key,
    query: Operation,
    tx: Sender,
    cancel: CancellationToken,
    limits: WebSocketLimits,
) -> End {
    let _cancel_on_exit = cancel.clone().drop_guard();
    if let Operation::Request(request) = query {
        let work = async {
            match request {
                crate::models::ApiRequest::Valuation { params, .. } => {
                    market.gift_valuation(*params).await.map(Payload::Result)
                }
            }
        };
        let result = tokio::select! {biased;
            _=cancel.cancelled()=>return End::Cancelled,
            result=tokio::time::timeout(limits.request_timeout,work)=>result.unwrap_or_else(|_|Err(ApiError::timed_out("Request timed out"))),
        };
        let payload = result.unwrap_or_else(Payload::Error);
        return end(tx.send(Scope::Operation(key), payload, true, &cancel).await);
    }
    let setup = async {
        match query {
            Operation::Request(_) => unreachable!("requests handled above"),
            Operation::Window(query) => market.subscribe_window(*query).await,
            Operation::Catalog(ids) => market.subscribe_catalog(ids).await,
        }
    };
    let result = tokio::select! {biased;
        _=cancel.cancelled()=>return End::Cancelled,
        result=tokio::time::timeout(limits.setup_timeout,setup)=>result.unwrap_or_else(|_|Err(ApiError::unavailable("Subscription setup timed out"))),
    };
    let (_lease, mut updates) = match result {
        Ok(subscription) => subscription,
        Err(error) => {
            return end(tx
                .send(Scope::Operation(key), Payload::Error(error), true, &cancel)
                .await);
        }
    };
    let mut previous_window = None;
    loop {
        let update = updates.borrow_and_update().clone();
        let (payload, terminal) = match update {
            DataUpdate::Connecting => (Payload::Connecting, false),
            DataUpdate::Catalog(versions) => (Payload::Catalog(versions), false),
            DataUpdate::Error(error) => (Payload::Error(error.into()), true),
            DataUpdate::Window {
                window,
                sync,
                revision,
            } => {
                if previous_window
                    .as_ref()
                    .is_some_and(|old| std::sync::Arc::ptr_eq(old, &window))
                {
                    (Payload::Sync { sync, revision }, false)
                } else {
                    previous_window = Some(window.clone());
                    let result = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,result=market.encode_window(window)=>result};
                    match result {
                        Ok(window) => (
                            Payload::Window {
                                window,
                                sync,
                                revision,
                            },
                            false,
                        ),
                        Err(error) => (Payload::Error(error), true),
                    }
                }
            }
        };
        let delivered = tx
            .send(Scope::Operation(key.clone()), payload, terminal, &cancel)
            .await;
        if delivered.is_err() || terminal {
            return end(delivered);
        }
        let changed = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,result=updates.changed()=>result};
        if changed.is_err() {
            return end(tx
                .send(
                    Scope::Operation(key),
                    Payload::Error(ApiError::unavailable("Subscription feed closed")),
                    true,
                    &cancel,
                )
                .await);
        }
    }
}
