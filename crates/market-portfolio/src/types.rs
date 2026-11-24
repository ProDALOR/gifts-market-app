use market_gifts::CurrencyId;
pub use market_gifts::inventory::*;
pub use market_valuation::InventoryValuation;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[cfg(feature = "schema")]
use utoipa::ToSchema;

const MAX_USER_ID: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct UserId(u64);
#[cfg(feature = "schema")]
impl utoipa::PartialSchema for UserId {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Integer)
            .minimum(Some(1u64))
            .maximum(Some(MAX_USER_ID))
            .into()
    }
}
#[cfg(feature = "schema")]
impl ToSchema for UserId {}
impl UserId {
    pub fn new(value: u64) -> Result<Self, PortfolioError> {
        if !(1..=MAX_USER_ID).contains(&value) {
            return Err(PortfolioError::InvalidData(
                "invalid Telegram user ID".into(),
            ));
        }
        Ok(Self(value))
    }
    pub fn get(self) -> u64 {
        self.0
    }
}
impl TryFrom<u64> for UserId {
    type Error = PortfolioError;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<UserId> for u64 {
    fn from(value: UserId) -> u64 {
        value.0
    }
}
#[derive(Debug, Clone)]
pub struct InventoryPageRequest {
    pub user: UserId,
    pub cursor: Option<String>,
    pub limit: usize,
}
#[cfg_attr(feature = "schema", derive(ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioQuery {
    pub user_id: UserId,
    pub currency_id: CurrencyId,
}
#[cfg_attr(feature = "schema", derive(ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioStage {
    Idle,
    Queued,
    LoadingInventory,
    Valuing,
    Ready,
    Failed,
}
#[cfg_attr(feature = "schema", derive(ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioProgress {
    pub stage: PortfolioStage,
    pub checked: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    pub valued: usize,
    pub collectibles: usize,
}
impl Default for PortfolioProgress {
    fn default() -> Self {
        Self {
            stage: PortfolioStage::Idle,
            checked: 0,
            total: None,
            valued: 0,
            collectibles: 0,
        }
    }
}
#[cfg_attr(feature = "schema", derive(ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventorySummary {
    #[cfg_attr(feature="schema", schema(value_type=String, format=DateTime))]
    pub checked_at: jiff::Timestamp,
    pub count: usize,
    pub scanned: usize,
    pub regular: usize,
    pub skipped: usize,
    pub complete: bool,
}
#[cfg_attr(feature = "schema", derive(ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioState {
    pub user_id: UserId,
    pub refresh_id: u64,
    pub progress: PortfolioProgress,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory: Option<InventorySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valuation: Option<Arc<InventoryValuation>>,
    /// Stable while item data/order is unchanged, independent of progress and check times.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<crate::PortfolioVersion>,
    #[cfg_attr(feature="schema", schema(value_type=Option<String>, format=DateTime))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_after: Option<jiff::Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Debug, Clone)]
pub enum PortfolioError {
    RestartRead,
    Busy,
    Stopped,
    TimedOut,
    InvalidConfig(&'static str),
    InvalidData(String),
    Inventory(InventoryError),
    Cooldown(u64),
}
impl std::fmt::Display for PortfolioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RestartRead => f.write_str("Portfolio changed; restart the paginated read"),
            Self::Busy => f.write_str("Portfolio capacity reached"),
            Self::Stopped => f.write_str("Portfolio service unavailable"),
            Self::TimedOut => f.write_str("Portfolio request timed out"),
            Self::InvalidConfig(value) => f.write_str(value),
            Self::InvalidData(value) => f.write_str(value),
            Self::Cooldown(seconds) => write!(f, "Gift refresh is available in {seconds}s"),
            Self::Inventory(InventoryError::Forbidden) => f.write_str("This user is not known to the connected Telegram session. Open the bot first or use a known public peer."),
            Self::Inventory(_) => f.write_str("Telegram inventory is temporarily unavailable"),
        }
    }
}
impl std::error::Error for PortfolioError {}
