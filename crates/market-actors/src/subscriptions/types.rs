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
    Window {
        window: Arc<market_gifts::OrderWindow>,
        sync: market_gifts::SyncStatus,
        revision: String,
    },
    Error(DataError),
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct SubscriberId(pub u64);
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum QueryKey {
    Catalog(Vec<market_gifts::CollectionId>),
    Head {
        request: Vec<u8>,
        before: u32,
        after: u32,
    },
    View(SubscriberId),
}
pub(super) enum FeedSpec {
    Catalog(Vec<market_gifts::CollectionId>),
    Window {
        request: Box<market_hub_protocol::wire::ReadRequest>,
        window: market_replica::WindowRequest,
    },
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
