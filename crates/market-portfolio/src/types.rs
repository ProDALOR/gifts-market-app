use market_actors::error::DataError;
use market_gifts::{CurrencyId, GiftId, TelegramDocumentId, TelegramGiftId, valuation::Valuation};
use std::{collections::BTreeMap, fmt, sync::Arc, time::Duration};

/// Canonical source user identity. Authorization belongs to the calling application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(u64);
impl UserId {
    pub fn new(value: u64) -> Result<Self, PortfolioError> {
        if value == 0 || value > i64::MAX as u64 {
            return Err(PortfolioError::InvalidData(
                "user ID must be a positive signed 64-bit integer".into(),
            ));
        }
        Ok(Self(value))
    }
    pub fn get(self) -> u64 {
        self.0
    }
}
pub use market_invocations::Priority;
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
/// One bounded provider operation. The actor owns pagination and request pacing.
#[derive(Debug, Clone)]
pub struct InventoryPageRequest {
    pub user: UserId,
    pub cursor: Option<String>,
    pub limit: usize,
}
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
#[derive(Debug, Clone)]
pub struct Portfolio {
    pub user: UserId,
    pub inventory: Arc<InventorySnapshot>,
    pub valuation: Arc<Valuation>,
}
#[derive(Debug, Clone)]
pub struct PortfolioRequest {
    pub user: UserId,
    pub currency_id: CurrencyId,
    pub priority: Priority,
}
#[derive(Debug, Clone)]
pub enum PortfolioError {
    Busy,
    Stopped,
    TimedOut,
    InvalidConfig(&'static str),
    InvalidData(String),
    Inventory(InventoryError),
    Valuation(DataError),
}
impl fmt::Display for PortfolioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => f.write_str("portfolio capacity reached"),
            Self::Stopped => f.write_str("portfolio service stopped"),
            Self::TimedOut => f.write_str("portfolio request timed out"),
            Self::InvalidConfig(message) => f.write_str(message),
            Self::InvalidData(message) => f.write_str(message),
            Self::Inventory(error) => write!(f, "inventory unavailable: {error:?}"),
            Self::Valuation(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for PortfolioError {}
