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
        observed: u64,
    },
    Error(DataError),
}
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum QueryKey {
    Catalog(Vec<market_gifts::CollectionId>),
    Range {
        options: market_replica::ReadOptions,
        selection: Vec<u8>,
        sort: i32,
        range: Box<market_replica::KeyRange>,
    },
}
pub(super) struct OrdersFeed {
    pub request: market_hub_protocol::wire::ReadRequest,
    pub options: market_replica::ReadOptions,
    pub initial: Arc<crate::OrderResult>,
    pub version: market_replica::ReadRevision,
    pub observed: u64,
}
pub(super) enum FeedSpec {
    Catalog(Vec<market_gifts::CollectionId>),
    Orders(Box<OrdersFeed>),
}
/// An immutable per-caller baseline, followed by coalesced updates from a shared producer.
/// Hold the lease while consuming updates. Process the current receiver value after the baseline.
pub struct DataWatch {
    pub lease: Arc<DataSubscription>,
    pub initial: DataUpdate,
    pub updates: watch::Receiver<DataUpdate>,
}
impl DataWatch {
    pub(super) fn new(feed: Arc<QueryFeed>, mut initial: DataUpdate) -> Self {
        let updates = feed.sender.subscribe();
        // Reuse the producer's canonical row references when this seed has the same state.
        if let DataUpdate::Orders { result, .. } = &mut initial
            && let DataUpdate::Orders {
                result: current, ..
            } = &*feed.sender.borrow()
            && (Arc::ptr_eq(result, current) || **result == **current)
        {
            *result = current.clone();
        }
        Self {
            lease: Arc::new(DataSubscription { _feed: feed }),
            initial,
            updates,
        }
    }
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

/// Publish in observation order, including removals whose scoped revision can fall back to zero.
/// The channel is the sole authority for both the last state and its observation clock.
pub(super) fn publish_orders(sender: &watch::Sender<DataUpdate>, incoming: DataUpdate) -> bool {
    let DataUpdate::Orders {
        result,
        version,
        observed,
    } = incoming
    else {
        unreachable!("order update required")
    };
    sender.send_if_modified(|state| {
        let DataUpdate::Orders {
            result: previous,
            version: old,
            observed: clock,
        } = state
        else {
            return false;
        };
        if observed < *clock {
            return false;
        }
        let rows_changed = !Arc::ptr_eq(previous, &result) && **previous != *result;
        let changed = version != *old || rows_changed;
        if rows_changed {
            *previous = result;
        }
        *old = version;
        *clock = observed;
        changed
    })
}
