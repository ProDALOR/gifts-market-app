//! Cancellable typed requests and subscription forwarding, independent of session I/O.
use super::{
    queue::{Payload, QueueError, Scope, Sender},
    registry::Key,
};
use crate::{MarketApi, config::WebSocketLimits, error::ApiError};
use market_actors::DataUpdate;
use tokio_util::sync::CancellationToken;
pub(super) enum Operation {
    Portfolio(market_portfolio::PortfolioQuery),
    Valuation(Box<market_valuation::ValuationQuery>),
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
    if let Operation::Portfolio(query) = query {
        let setup = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,result=market.portfolio_watch(query)=>result};
        let mut watch = match setup {
            Ok(w) => w,
            Err(e) => {
                return end(tx
                    .send(Scope::Operation(key), Payload::Error(e), true, &cancel)
                    .await);
            }
        };
        let mut value = watch.current();
        let mut previous: Option<std::sync::Arc<market_portfolio::PortfolioState>> = None;
        loop {
            let progress =
                previous
                    .as_ref()
                    .is_some_and(|p| match (&p.valuation, &value.valuation) {
                        (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                        (None, None) => true,
                        _ => false,
                    })
                    && previous
                        .as_ref()
                        .and_then(|p| p.inventory.as_ref().map(|i| i.checked_at))
                        == value.inventory.as_ref().map(|i| i.checked_at);
            let envelope = if progress {
                crate::encoding::PortfolioEnvelope::Progress(key.id.as_str().into())
            } else {
                crate::encoding::PortfolioEnvelope::Snapshot(key.id.as_str().into())
            };
            let encoded = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,v=market.encode_portfolio(value.clone(),envelope)=>v};
            let encoded = match encoded {
                Ok(v) => v,
                Err(e) => {
                    return end(tx
                        .send(Scope::Operation(key), Payload::Error(e), true, &cancel)
                        .await);
                }
            };
            if let Err(e) = tx
                .send(
                    Scope::Operation(key.clone()),
                    Payload::Encoded(encoded),
                    false,
                    &cancel,
                )
                .await
            {
                return end(Err(e));
            }
            previous = Some(value);
            value = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,v=watch.changed()=>match v{Ok(v)=>v,Err(e)=>return end(tx.send(Scope::Operation(key),Payload::Error(ApiError::unavailable(e.to_string())),true,&cancel).await)}};
        }
    }
    if let Operation::Valuation(request) = query {
        let setup = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,result=tokio::time::timeout(limits.setup_timeout,market.watch_valuation(*request))=>result.unwrap_or_else(|_|Err(ApiError::timed_out("Valuation watch setup timed out")))};
        let mut watch = match setup {
            Ok(watch) => watch,
            Err(error) => {
                return end(tx
                    .send(Scope::Operation(key), Payload::Error(error), true, &cancel)
                    .await);
            }
        };
        let mut value = watch.current();
        loop {
            let encoded = tokio::select! { biased; _=cancel.cancelled()=>return End::Cancelled, result=market.encode_valuation(value, crate::encoding::ValuationEnvelope::Snapshot(key.id.as_str().into()))=>result };
            let encoded = match encoded {
                Ok(value) => value,
                Err(error) => {
                    return end(tx
                        .send(Scope::Operation(key), Payload::Error(error), true, &cancel)
                        .await);
                }
            };
            if let Err(error) = tx
                .send(
                    Scope::Operation(key.clone()),
                    Payload::Encoded(encoded),
                    false,
                    &cancel,
                )
                .await
            {
                return end(Err(error));
            }
            let changed = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,value=watch.changed()=>value};
            match changed {
                Ok(next) => value = next,
                Err(error) => {
                    return end(tx
                        .send(
                            Scope::Operation(key),
                            Payload::Error(error.into()),
                            true,
                            &cancel,
                        )
                        .await);
                }
            }
        }
    }
    if let Operation::Request(request) = query {
        let work = async {
            match request {
                crate::models::ApiRequest::PortfolioQuery { id, params } => {
                    let data = market.portfolio_query(params).await?;
                    market
                        .encode_portfolio(data, crate::encoding::PortfolioEnvelope::Query(id))
                        .await
                        .map(Payload::Encoded)
                }
                crate::models::ApiRequest::PortfolioRefresh { id, params } => {
                    let data = market.portfolio_refresh(params).await?;
                    market
                        .encode_portfolio(data, crate::encoding::PortfolioEnvelope::Refresh(id))
                        .await
                        .map(Payload::Encoded)
                }
                crate::models::ApiRequest::Valuation { id, params } => {
                    let report = market.valuation(*params).await?;
                    market
                        .encode_valuation(report, crate::encoding::ValuationEnvelope::Query(id))
                        .await
                        .map(Payload::Encoded)
                }
                crate::models::ApiRequest::Orders { id, params } => {
                    let read = market.query_orders(*params).await?;
                    market
                        .encode_orders(crate::encoding::OrderEvent::Read {
                            id,
                            read: Box::new(read),
                        })
                        .await
                        .map(Payload::Encoded)
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
            Operation::Portfolio(_) => unreachable!("portfolios handled above"),
            Operation::Valuation(_) => unreachable!("values handled above"),
            Operation::Request(_) => unreachable!("requests handled above"),
            Operation::Orders(query) => market.watch_orders(*query).await,
            Operation::Catalog(ids) => market.subscribe_catalog(ids).await,
        }
    };
    let result = tokio::select! {biased;
        _=cancel.cancelled()=>return End::Cancelled,
        result=tokio::time::timeout(limits.setup_timeout,setup)=>result.unwrap_or_else(|_|Err(ApiError::unavailable("Subscription setup timed out"))),
    };
    let market_actors::DataWatch {
        lease: _lease,
        initial,
        mut updates,
    } = match result {
        Ok(subscription) => subscription,
        Err(error) => {
            return end(tx
                .send(Scope::Operation(key), Payload::Error(error), true, &cancel)
                .await);
        }
    };
    let mut initial = Some(initial);
    let mut previous = None;
    let mut epoch: Option<market_actors::ReadRevision> = None;
    let mut sequence = 0u64;
    loop {
        let first = initial.is_some();
        let update = initial
            .take()
            .unwrap_or_else(|| updates.borrow_and_update().clone());
        let message = match update {
            DataUpdate::Connecting => None,
            DataUpdate::Catalog(versions) => Some((Payload::Catalog(versions), false)),
            DataUpdate::Error(error) => Some((Payload::Error(error.into()), true)),
            DataUpdate::Orders {
                result,
                version: next_epoch,
                ..
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
                        sequence: next_sequence,
                        reset,
                    };
                    let encoded = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,result=market.encode_orders(event)=>result};
                    match encoded {
                        Ok(encoded) => {
                            previous = Some(result);
                            epoch = Some(next_epoch);
                            sequence = next_sequence;
                            Some((Payload::Encoded(encoded), false))
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
        if first {
            continue;
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
