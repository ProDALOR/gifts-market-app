//! Shared gift valuation requests and failure metadata.
mod metric;
use market_gifts::{CurrencyId, InvalidValue};
pub use metric::*;
use serde::{Deserialize, Serialize};

/// Stable strategy name; unknown valid names are rejected by the running registry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature="schema",schema(value_type=String,pattern="^[a-z][a-z0-9_]{0,63}$",example="gift_floor"))]
pub struct StrategyId(String);
impl StrategyId {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value.as_bytes()[0].is_ascii_lowercase()
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(InvalidValue(
                "strategy must contain 1..64 lowercase letters, digits or underscores and start with a letter",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Display for StrategyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl TryFrom<String> for StrategyId {
    type Error = InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<StrategyId> for String {
    fn from(value: StrategyId) -> Self {
        value.0
    }
}
/// Shared failure metadata with typed, operation-specific evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationErrorDetails {
    pub strategy: StrategyId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency_id: Option<CurrencyId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_samples: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_samples: Option<u32>,
    #[cfg(feature = "portfolio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<crate::ValuationErrorContext>,
}
impl ValuationErrorDetails {
    pub fn strategy(strategy: StrategyId) -> Self {
        Self {
            strategy,
            currency_id: None,
            revision: None,
            required_samples: None,
            available_samples: None,
            #[cfg(feature = "portfolio")]
            context: None,
        }
    }
}

/// Evaluate one concrete gift using the unified floor strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftValuationRequest {
    pub gift: market_gifts::GiftLookup,
    pub currency_id: CurrencyId,
}

#[cfg(feature = "portfolio")]
impl StrategyId {
    pub fn model_floor() -> Self {
        Self("model_floor".into())
    }
    pub fn collection_floor() -> Self {
        Self("collection_floor".into())
    }
}

#[cfg(test)]
mod tests;
