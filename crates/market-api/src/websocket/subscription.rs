//! One cancellable setup-and-forward future per subscription generation.
use super::{
    queue::{Payload, QueueError, Scope, Sender},
    registry::Key,
};
use crate::{MarketApi, config::WebSocketLimits, error::ApiError, models::GiftOrdersQuery};
use market_actors::DataUpdate;
use tokio_util::sync::CancellationToken;
pub(super) enum Query {
    Orders(Box<GiftOrdersQuery>),
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
    query: Query,
    tx: Sender,
    cancel: CancellationToken,
    limits: WebSocketLimits,
) -> End {
    let _cancel_on_exit = cancel.clone().drop_guard();
    let setup = async {
        match query {
            Query::Orders(query) => market.subscribe(*query).await,
            Query::Catalog(ids) => market.subscribe_catalog(ids).await,
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
                .send(
                    Scope::Subscription(key),
                    Payload::Error(error),
                    true,
                    &cancel,
                )
                .await);
        }
    };
    let mut previous = None;
    loop {
        let update = updates.borrow_and_update().clone();
        let (payload, terminal) = match update {
            DataUpdate::Connecting => (Payload::Connecting, false),
            DataUpdate::Catalog(versions) => (Payload::Catalog(versions), false),
            DataUpdate::Error(error) => (Payload::Error(error.into()), true),
            DataUpdate::View {
                page,
                sync,
                revision,
            } => {
                if previous
                    .as_ref()
                    .is_some_and(|old| std::sync::Arc::ptr_eq(old, &page))
                {
                    (Payload::Sync { sync, revision }, false)
                } else {
                    previous = Some(page.clone());
                    let result = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,result=market.encode_page(page)=>result};
                    match result {
                        Ok(page) => (
                            Payload::Page {
                                page,
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
            .send(Scope::Subscription(key.clone()), payload, terminal, &cancel)
            .await;
        if delivered.is_err() || terminal {
            return end(delivered);
        }
        let changed = tokio::select! {biased;_=cancel.cancelled()=>return End::Cancelled,result=updates.changed()=>result};
        if changed.is_err() {
            return end(tx
                .send(
                    Scope::Subscription(key),
                    Payload::Error(ApiError::unavailable("Subscription feed closed")),
                    true,
                    &cancel,
                )
                .await);
        }
    }
}
