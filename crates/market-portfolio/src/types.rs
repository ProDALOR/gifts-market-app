use market_gifts::CurrencyId;
use market_valuation::Valuation;
use market_valuation::ValuationError;
use std::{fmt, sync::Arc};

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
pub use market_gifts::inventory::*;
pub use market_invocations::Priority;
#[derive(Debug, Clone)]
pub struct InventoryPageRequest {
    pub user: UserId,
    pub cursor: Option<String>,
    pub limit: usize,
}
#[derive(Debug, Clone)]
pub struct Portfolio {
    pub user: UserId,
    pub inventory: Arc<InventorySnapshot>,
    pub valuation: Arc<market_valuation::Mark<Valuation>>,
}
#[derive(Debug, Clone)]
pub struct PortfolioRequest {
    pub strategy: market_valuation::StrategyId,
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
    Valuation(ValuationError),
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
