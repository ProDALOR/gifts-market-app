use super::{MarketSubscriptionsActor, messages::Release};
use crate::error::DataError;
use actix::Addr;
use std::sync::{Arc, atomic::AtomicBool};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub enum DataUpdate {
    Connecting,
    Catalog(market_gifts::CatalogVersions),
    Orders {
        result: Arc<crate::OrderResult>,
        version: market_replica::ReadRevision,
    },
    Error(DataError),
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct SubscriberId(pub u64);
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum QueryKey {
    Catalog(Vec<market_gifts::CollectionId>),
    View(SubscriberId),
    Range {
        selection: Vec<u8>,
        sort: i32,
        range: Box<market_replica::KeyRange>,
    },
}
pub(super) struct OrdersFeed {
    pub range: Option<market_replica::KeyRange>,
    pub request: market_hub_protocol::wire::ReadRequest,
    pub slice: market_replica::SliceRequest,
}
pub(super) enum FeedSpec {
    Catalog(Vec<market_gifts::CollectionId>),
    Orders(Box<OrdersFeed>),
}
/// One caller's lease. Dropping its final Arc releases only that caller.
pub struct DataSubscription {
    pub(super) id: SubscriberId,
    pub(super) owner: Addr<MarketSubscriptionsActor>,
}
impl Drop for DataSubscription {
    fn drop(&mut self) {
        // Cleanup must be delivered even when the ordinary request mailbox is full.
        self.owner.do_send(Release(self.id));
    }
}
pub(super) struct QueryFeed {
    pub sender: watch::Sender<DataUpdate>,
    pub done: AtomicBool,
    pub stop: CancellationToken,
}
