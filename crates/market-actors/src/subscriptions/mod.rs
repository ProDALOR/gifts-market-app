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
pub use types::{DataSubscription, DataUpdate, DataWatch};
use types::{FeedSpec, QueryFeed, QueryKey};
pub(crate) struct MarketSubscriptionsActor {
    environment: ActorEnvironment,
    orders: OrderbooksHandle,
    catalog: CatalogHandle,
    reader: market_replica::ReplicaReader,
    slots: Arc<Semaphore>,
    seeds: Arc<Semaphore>,
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
            seeds: Arc::new(Semaphore::new(limit)),
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
    type Result = ResponseFuture<Result<DataWatch, Status>>;
    fn handle(&mut self, message: WatchOrders, ctx: &mut Self::Context) -> Self::Result {
        if message.cancel.is_cancelled() || self.environment.stop.is_cancelled() {
            return Box::pin(async { Err(Status::cancelled("Watch cancelled")) });
        }
        // An up-to-date feed covering the exact requested bounds already is a valid seed.
        let key = range_key(&message.request, &message.read.range, message.read.options);
        if let Some(feed) = self
            .feeds
            .get(&key)
            .and_then(Weak::upgrade)
            .filter(|feed| feed.reusable())
        {
            let initial = feed.sender.borrow().clone();
            if let DataUpdate::Orders {
                result, version, ..
            } = &initial
                && result.data.len() <= message.read.limit as usize
                && self
                    .reader
                    .read_revision(&message.request)
                    .is_ok_and(|current| current == *version)
            {
                let watch = DataWatch::new(feed, initial);
                return Box::pin(async move { Ok(watch) });
            }
        }
        let Ok(permit) = self.seeds.clone().try_acquire_owned() else {
            return Box::pin(async { Err(capacity()) });
        };
        let orders = self.orders.clone();
        let address = ctx.address();
        let stop = self.environment.stop.clone();
        let options = message.read.options;
        let job = self.environment.jobs.spawn_on(async move {
            let _permit = permit;
            // Each caller initializes against current data. Only resolved coverage can share a feed.
            let initial = tokio::select! { biased;
                _ = stop.cancelled() => return Err(Status::unavailable("Market subscriptions are stopping")),
                _ = message.cancel.cancelled() => return Err(Status::cancelled("Watch cancelled")),
                result = orders.range_wait(message.request.clone(), message.read, false) => result?,
            };
            tokio::select! { biased;
                _ = stop.cancelled() => Err(Status::unavailable("Market subscriptions are stopping")),
                _ = message.cancel.cancelled() => Err(Status::cancelled("Watch cancelled")),
                result = address.send(messages::RegisterOrders {
                    feed: types::OrdersFeed {
                        request: message.request,
                        options,
                        initial: Arc::new(initial.read.result),
                        version: initial.version,
                        observed: initial.observed,
                    },
                    cancel: message.cancel.clone(),
                }) => result.map_err(crate::runtime::closed)?,
            }
        }, &self.environment.runtime);
        Box::pin(async move {
            job.await
                .map_err(|_| Status::unavailable("Watch initialization stopped"))?
        })
    }
}
impl Handler<messages::RegisterOrders> for MarketSubscriptionsActor {
    type Result = Result<DataWatch, Status>;
    fn handle(&mut self, message: messages::RegisterOrders, _: &mut Self::Context) -> Self::Result {
        if message.cancel.is_cancelled() {
            return Err(Status::cancelled("Watch cancelled"));
        }
        self.register(FeedSpec::Orders(Box::new(message.feed)))
    }
}
impl Handler<SubscribeCatalog> for MarketSubscriptionsActor {
    type Result = Result<DataWatch, Status>;
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
    fn register(&mut self, spec: FeedSpec) -> Result<DataWatch, Status> {
        if self.environment.stop.is_cancelled() {
            return Err(Status::unavailable("Market subscriptions are stopping"));
        }
        let key = match &spec {
            FeedSpec::Catalog(ids) => QueryKey::Catalog(ids.clone()),
            FeedSpec::Orders(selection) => range_key(
                &selection.request,
                &selection.initial.range,
                selection.options,
            ),
        };
        let initial = match &spec {
            FeedSpec::Orders(seed) => DataUpdate::Orders {
                result: seed.initial.clone(),
                version: seed.version,
                observed: seed.observed,
            },
            FeedSpec::Catalog(_) => DataUpdate::Connecting,
        };
        if let Some(feed) = self
            .feeds
            .get(&key)
            .and_then(Weak::upgrade)
            .filter(|feed| feed.reusable())
        {
            return Ok(match &spec {
                FeedSpec::Orders(_) => {
                    types::publish_orders(&feed.sender, initial.clone());
                    DataWatch::new(feed, initial)
                }
                FeedSpec::Catalog(_) => DataWatch::new(feed, DataUpdate::Connecting),
            });
        }

        // Shared joins stay O(1); new producers prune weak entries before admission.
        self.feeds
            .retain(|_, entry| entry.upgrade().is_some_and(|feed| feed.reusable()));
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| capacity())?;
        let (sender, _updates) = watch::channel(initial.clone());
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
                selection.request,
                selection.options,
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
        self.feeds.insert(key, Arc::downgrade(&feed));
        Ok(DataWatch::new(feed, initial))
    }
}

fn range_key(
    request: &market_hub_protocol::wire::ReadRequest,
    range: &market_replica::KeyRange,
    options: market_replica::ReadOptions,
) -> QueryKey {
    QueryKey::Range {
        options: options.normalized_for(request),
        selection: request
            .selection
            .as_ref()
            .map(|selection| selection.encode_to_vec())
            .unwrap_or_default(),
        sort: request.sort,
        range: Box::new(range.clone()),
    }
}

#[cfg(test)]
mod tests;
