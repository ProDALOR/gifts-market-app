//! Per-caller subscription leases and deduplicated live orderbook feeds.
pub(crate) mod messages;
mod producer;
mod types;
use crate::{
    orderbooks::handle::OrderbooksHandle,
    runtime::{ActorEnvironment, capacity},
    selection,
};
use actix::prelude::*;
use messages::{Release, Subscribe};
use prost::Message;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Semaphore, watch};
use tonic::Status;
pub use types::{DataSubscription, DataUpdate};
use types::{QueryFeed, QueryKey, SubscriberId};
struct FeedEntry {
    feed: Arc<QueryFeed>,
    subscribers: HashSet<SubscriberId>,
}
pub(crate) struct UserSubscriptionsActor {
    environment: ActorEnvironment,
    orders: OrderbooksHandle,
    changed: watch::Receiver<u64>,
    slots: Arc<Semaphore>,
    feeds: HashMap<QueryKey, FeedEntry>,
    subscribers: HashMap<SubscriberId, QueryKey>,
    next_id: u64,
}
impl UserSubscriptionsActor {
    pub fn new(
        environment: ActorEnvironment,
        orders: OrderbooksHandle,
        changed: watch::Receiver<u64>,
        limit: usize,
    ) -> Self {
        Self {
            environment,
            orders,
            changed,
            slots: Arc::new(Semaphore::new(limit)),
            feeds: HashMap::new(),
            subscribers: HashMap::new(),
            next_id: 0,
        }
    }
}
impl Actor for UserSubscriptionsActor {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.environment.started(ctx);
    }
}
impl Drop for UserSubscriptionsActor {
    fn drop(&mut self) {
        self.environment.stopped();
    }
}
impl Handler<Subscribe> for UserSubscriptionsActor {
    type Result = Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>), Status>;
    fn handle(&mut self, message: Subscribe, ctx: &mut Self::Context) -> Self::Result {
        selection::orders(&message.0)?;
        let next = self.next_id.checked_add(1).ok_or_else(capacity)?;
        let id = SubscriberId(next);
        let key = QueryKey(message.0.encode_to_vec());
        let running = self
            .feeds
            .get(&key)
            .is_some_and(|entry| !entry.feed.done.load(Ordering::Acquire));
        if !running {
            let permit = self
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| capacity())?;
            let (sender, _) = watch::channel(DataUpdate::Connecting);
            let feed = Arc::new(QueryFeed {
                sender,
                done: AtomicBool::new(false),
                stop: self.environment.stop.child_token(),
            });
            // Register the receiver before starting the producer so it cannot observe a closed channel.
            let updates = feed.sender.subscribe();
            producer::start(
                &self.environment,
                &feed,
                self.orders.clone(),
                message.0,
                self.changed.clone(),
                permit,
            );
            self.feeds.insert(
                key.clone(),
                FeedEntry {
                    feed,
                    subscribers: [id].into(),
                },
            );
            self.subscribers.insert(id, key);
            self.next_id = next;
            return Ok((
                Arc::new(DataSubscription {
                    id,
                    owner: ctx.address(),
                }),
                updates,
            ));
        }
        let entry = self.feeds.get_mut(&key).expect("running feed exists");
        entry.subscribers.insert(id);
        let updates = entry.feed.sender.subscribe();
        self.subscribers.insert(id, key);
        self.next_id = next;
        Ok((
            Arc::new(DataSubscription {
                id,
                owner: ctx.address(),
            }),
            updates,
        ))
    }
}
impl Handler<Release> for UserSubscriptionsActor {
    type Result = ();
    fn handle(&mut self, message: Release, _: &mut Self::Context) {
        let Some(key) = self.subscribers.remove(&message.0) else {
            return;
        };
        if let Some(entry) = self.feeds.get_mut(&key) {
            entry.subscribers.remove(&message.0);
            if entry.subscribers.is_empty() {
                let entry = self.feeds.remove(&key).expect("feed exists");
                entry.feed.stop.cancel();
            }
        }
    }
}
