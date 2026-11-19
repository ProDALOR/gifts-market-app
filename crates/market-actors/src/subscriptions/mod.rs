//! Per-caller subscription leases and deduplicated live orderbook feeds.
pub(crate) mod messages;
mod producer;
mod types;
use crate::{
    catalog::handle::CatalogHandle,
    orderbooks::handle::OrderbooksHandle,
    runtime::{ActorEnvironment, capacity},
};
use actix::prelude::*;
use messages::{SubscribeCatalog, WatchOrders};
use prost::Message as _;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use tokio::sync::{Semaphore, watch};
use tonic::Status;
pub use types::{DataSubscription, DataUpdate};
use types::{FeedSpec, QueryFeed, QueryKey};
pub(crate) struct MarketSubscriptionsActor {
    environment: ActorEnvironment,
    orders: OrderbooksHandle,
    catalog: CatalogHandle,
    reader: market_replica::ReplicaReader,
    slots: Arc<Semaphore>,
    feeds: HashMap<QueryKey, Weak<QueryFeed>>,
}
impl MarketSubscriptionsActor {
    pub fn new(
        environment: ActorEnvironment,
        orders: OrderbooksHandle,
        catalog: CatalogHandle,
        reader: market_replica::ReplicaReader,
        limit: usize,
    ) -> Self {
        Self {
            environment,
            orders,
            catalog,
            reader,
            slots: Arc::new(Semaphore::new(limit)),
            feeds: HashMap::new(),
        }
    }
}
impl Actor for MarketSubscriptionsActor {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.environment.started(ctx);
    }
}
impl Drop for MarketSubscriptionsActor {
    fn drop(&mut self) {
        self.environment.stopped();
    }
}
impl Handler<WatchOrders> for MarketSubscriptionsActor {
    type Result = Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>), Status>;
    fn handle(&mut self, message: WatchOrders, _: &mut Self::Context) -> Self::Result {
        self.register(FeedSpec::Orders(Box::new(types::OrdersFeed {
            request: message.request,
            read: message.read,
        })))
    }
}
impl Handler<SubscribeCatalog> for MarketSubscriptionsActor {
    type Result = Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>), Status>;
    fn handle(&mut self, mut message: SubscribeCatalog, _: &mut Self::Context) -> Self::Result {
        if message.0.len() > 8 {
            return Err(Status::invalid_argument("Watch at most eight collections"));
        }
        message.0.sort();
        message.0.dedup();
        self.register(FeedSpec::Catalog(message.0))
    }
}
impl MarketSubscriptionsActor {
    fn register(
        &mut self,
        spec: FeedSpec,
    ) -> Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>), Status> {
        if self.environment.stop.is_cancelled() {
            return Err(Status::unavailable("Market subscriptions are stopping"));
        }
        let key = match &spec {
            FeedSpec::Catalog(ids) => Some(QueryKey::Catalog(ids.clone())),
            FeedSpec::Orders(selection) => Some(QueryKey::Range {
                selection: selection
                    .request
                    .selection
                    .as_ref()
                    .map(|s| s.encode_to_vec())
                    .unwrap_or_default(),
                sort: selection.request.sort,
                range: Box::new(selection.read.range.clone()),
                limit: selection.read.limit,
            }),
        };
        if let Some(feed) = key
            .as_ref()
            .and_then(|key| self.feeds.get(key))
            .and_then(Weak::upgrade)
            .filter(|feed| feed.reusable())
        {
            let updates = feed.sender.subscribe();
            return Ok((Arc::new(DataSubscription { _feed: feed }), updates));
        }
        // Shared joins stay O(1); new producers prune weak entries before admission.
        self.feeds
            .retain(|_, entry| entry.upgrade().is_some_and(|feed| feed.reusable()));
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| capacity())?;
        let (sender, updates) = watch::channel(DataUpdate::Connecting);
        let feed = Arc::new(QueryFeed {
            sender,
            stop: self.environment.stop.child_token(),
        });
        // The initial receiver exists before the producer can observe channel closure.
        match spec {
            FeedSpec::Orders(selection) => producer::start_orders(
                &self.environment,
                &feed,
                self.orders.clone(),
                *selection,
                self.reader.clone(),
                permit,
            ),
            FeedSpec::Catalog(ids) => producer::start_catalog(
                &self.environment,
                &feed,
                self.catalog.clone(),
                ids,
                self.reader.clone(),
                permit,
            ),
        }
        if let Some(key) = key {
            self.feeds.insert(key, Arc::downgrade(&feed));
        }
        Ok((Arc::new(DataSubscription { _feed: feed }), updates))
    }
}

#[cfg(test)]
mod tests;
