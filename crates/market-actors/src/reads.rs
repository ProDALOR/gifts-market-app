/// Typed actor read results. Public encodings belong to the API transport.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderedEntry {
    pub order: market_gifts::GiftOrder,
    pub key: market_replica::ReadPosition,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OrderResult {
    pub data: Vec<OrderedEntry>,
    pub range: market_replica::KeyRange,
}
#[derive(Debug)]
pub struct OrderRead {
    pub result: OrderResult,
    pub snapshot: Option<market_gifts::ReadSnapshotId>,
}
pub use market_replica::{KeyRange, ReadPosition, ReadRevision};

/// One pinned actor read and its internal publication metadata.
pub(crate) struct OrderSnapshot {
    pub read: OrderRead,
    pub version: ReadRevision,
    pub observed: u64,
}
