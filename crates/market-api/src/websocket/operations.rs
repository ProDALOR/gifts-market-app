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
    Orders(Box<market_gifts::GiftOrderQuery>),
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
                crate::models::ApiRequest::Orders { id, params } => {
                    let read = market.query_orders(*params).await?;
                    market
                        .encode_orders(crate::encoding::OrderEvent::Read {
                            id,
                            read: Box::new(read),
                        })
                        .await
                        .map(Payload::Orders)
                }
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
            Operation::Orders(query) => market.watch_orders(*query).await,
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
    let mut previous = None;
    let mut epoch: Option<market_actors::ReadRevision> = None;
    let mut sequence = 0u64;
    loop {
        let update = updates.borrow_and_update().clone();
        let message = match update {
            DataUpdate::Connecting => None,
            DataUpdate::Catalog(versions) => Some((Payload::Catalog(versions), false)),
            DataUpdate::Error(error) => Some((Payload::Error(error.into()), true)),
            DataUpdate::Orders {
                result,
                next,
                version: next_epoch,
            } => {
                let reset = epoch
                    .as_ref()
                    .is_some_and(|old| !old.same_dataset(&next_epoch));
                if !reset
                    && previous
                        .as_ref()
                        .is_some_and(|old| std::sync::Arc::ptr_eq(old, &result))
                {
                    None
                } else {
                    let next_sequence = if previous.is_none() {
                        Some(0)
                    } else {
                        sequence
                            .checked_add(1)
                            .filter(|n| *n <= 9_007_199_254_740_991)
                    };
                    let Some(next_sequence) = next_sequence else {
                        return end(tx
                            .send(
                                Scope::Operation(key),
                                Payload::Error(ApiError::unavailable(
                                    "Subscription sequence exhausted; restart read",
                                )),
                                true,
                                &cancel,
                            )
                            .await);
                    };
                    let event = crate::encoding::OrderEvent::Update {
                        id: key.id.as_str().into(),
                        previous: previous.clone(),
                        current: result.clone(),
                        next,
                        sequence: next_sequence,
                        reset,
                    };
                    let encoded = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,result=market.encode_orders(event)=>result};
                    match encoded {
                        Ok(encoded) => {
                            previous = Some(result);
                            epoch = Some(next_epoch);
                            sequence = next_sequence;
                            Some((Payload::Orders(encoded), false))
                        }
                        Err(error) => Some((Payload::Error(error), true)),
                    }
                }
            }
        };
        if let Some((payload, terminal)) = message {
            let delivered = tx
                .send(Scope::Operation(key.clone()), payload, terminal, &cancel)
                .await;
            if delivered.is_err() || terminal {
                return end(delivered);
            }
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
