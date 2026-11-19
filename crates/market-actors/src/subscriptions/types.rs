use crate::error::DataError;
use std::sync::Arc;
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
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum QueryKey {
    Catalog(Vec<market_gifts::CollectionId>),
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
/// One caller's lease. Clones retain it; the final lease cancels the shared feed.
pub struct DataSubscription {
    pub(super) _feed: Arc<QueryFeed>,
}
pub(super) struct QueryFeed {
    pub sender: watch::Sender<DataUpdate>,
    pub stop: CancellationToken,
}
impl QueryFeed {
    pub fn reusable(&self) -> bool {
        !self.stop.is_cancelled()
            && !self.sender.is_closed()
            && !matches!(*self.sender.borrow(), DataUpdate::Error(_))
    }
}
impl Drop for QueryFeed {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
