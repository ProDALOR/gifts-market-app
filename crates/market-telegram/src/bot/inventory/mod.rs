//! Telegram profile data. Catalog aliases and valuation belong to command services.
mod collector;
pub(super) use collector::{InventoryPage, collect};
use std::{collections::BTreeMap, future::Future};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiftRelation {
    Owned,
    Hosted,
    Unverified,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileGift {
    pub id: i64,
    pub collection_id: i64,
    pub model_document_id: i64,
    pub relation: GiftRelation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkipReason {
    Refunded,
    Burned,
    OtherProfile,
    MissingGiftId,
    MissingCollection,
    MissingModel,
}
impl SkipReason {
    pub fn description(self) -> &'static str {
        match self {
            Self::Refunded => "refunded",
            Self::Burned => "burned",
            Self::OtherProfile => "owner or host does not match",
            Self::MissingGiftId => "missing gift ID",
            Self::MissingCollection => "missing collection ID",
            Self::MissingModel => "missing model document",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryError {
    Authorization,
    Unavailable,
    Forbidden,
    RetryAfter(u64),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartialReason {
    NotRead,
    LimitReached,
    TimedOut,
    MissingCursor,
    RepeatedCursor,
    ReadFailed(InventoryError),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryCompletion {
    Complete,
    Partial(PartialReason),
}
impl Default for InventoryCompletion {
    fn default() -> Self {
        Self::Partial(PartialReason::NotRead)
    }
}
#[derive(Debug, Clone, Default)]
pub struct VisibleInventory {
    pub gifts: Vec<ProfileGift>,
    pub regular_gifts: usize,
    pub skipped: BTreeMap<SkipReason, usize>,
    pub pages_read: usize,
    pub gifts_scanned: usize,
    pub reported_total: Option<usize>,
    pub completion: InventoryCompletion,
}
impl VisibleInventory {
    pub fn skipped_count(&self) -> usize {
        self.skipped.values().sum()
    }
    pub fn relation_count(&self, relation: GiftRelation) -> usize {
        self.gifts
            .iter()
            .filter(|gift| gift.relation == relation)
            .count()
    }
}
pub trait GiftSource: Sync {
    fn inventory(&self) -> impl Future<Output = Result<VisibleInventory, InventoryError>> + Send;
}
