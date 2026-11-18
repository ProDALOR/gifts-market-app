//! Source-normalized gift inventories shared by transport adapters and portfolio services.
use crate::{GiftId, TelegramDocumentId, TelegramGiftId};
use std::{collections::BTreeMap, time::Duration};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiftRelation {
    Owned,
    Hosted,
    Unverified,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryGift {
    pub id: GiftId,
    pub collection_id: TelegramGiftId,
    pub model_document_id: TelegramDocumentId,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryError {
    Authorization,
    Forbidden,
    Unavailable,
    RetryAfter(Duration),
    InvalidData(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartialReason {
    LimitReached,
    MissingCursor,
    RepeatedCursor,
    ReadFailed(InventoryError),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryCompletion {
    Complete,
    Partial(PartialReason),
}
/// One bounded inventory page; workflow actors own pagination.
#[derive(Debug, Clone, Default)]
pub struct InventoryPage {
    pub gifts: Vec<InventoryGift>,
    pub regular_gifts: usize,
    pub skipped: BTreeMap<SkipReason, usize>,
    pub scanned: usize,
    pub reported_total: Option<usize>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone)]
pub struct InventorySnapshot {
    pub gifts: Vec<InventoryGift>,
    pub regular_gifts: usize,
    pub skipped: BTreeMap<SkipReason, usize>,
    pub pages_read: usize,
    pub gifts_scanned: usize,
    pub reported_total: Option<usize>,
    pub completion: InventoryCompletion,
    pub fetched_at: jiff::Timestamp,
}

impl InventorySnapshot {
    pub fn skipped_count(&self) -> usize {
        self.skipped.values().sum()
    }
    pub fn relation_count(&self, relation: GiftRelation) -> usize {
        self.gifts.iter().filter(|g| g.relation == relation).count()
    }
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
