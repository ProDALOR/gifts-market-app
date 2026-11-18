//! Quantity-based portfolio contracts, enabled independently of gift valuation.
use crate::StrategyId;
use market_gifts::{
    Amount, CurrencyId, InvalidValue, ModelIdentity, ModelLookup, SyncStatus, VenueId,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ModelPosition {
    pub model: ModelLookup,
    pub quantity: u32,
}
impl ModelPosition {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if self.quantity == 0 {
            return Err(InvalidValue("portfolio quantities must be positive"));
        }
        self.model.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationRequest {
    pub strategy: StrategyId,
    pub positions: Vec<ModelPosition>,
    pub currency_id: CurrencyId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum UnpricedReason {
    UnknownCollection,
    AmbiguousCollection,
    UnknownModel,
    AmbiguousModel,
    NoListing,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ModelValue {
    /// Stable catalog identity; labels below remain useful for unresolved input.
    pub identity: Option<ModelIdentity>,
    pub collection: String,
    pub model: String,
    pub quantity: u32,
    pub floor: Option<Amount>,
    pub value: Option<Amount>,
    pub venue_id: Option<VenueId>,
    pub unpriced: Option<UnpricedReason>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Valuation {
    pub rows: Vec<ModelValue>,
    pub total: Amount,
    pub priced: u32,
    pub unpriced: u32,
    pub currency_id: CurrencyId,
    pub sync: SyncStatus,
}
impl ValuationRequest {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        let mut count = 0u32;
        for position in &self.positions {
            count = count
                .checked_add(position.quantity)
                .ok_or(InvalidValue("portfolio quantity is too large"))?;
            position.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioPositions {
    pub positions: Vec<ModelPosition>,
    pub currency_id: CurrencyId,
}
impl StrategyId {
    pub fn is_portfolio_strategy(&self) -> bool {
        matches!(self.as_str(), "model_floor" | "collection_floor")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationErrorContext {
    Portfolio { unpriced: Vec<ModelValue> },
}

impl From<market_gifts::ModelLookupError> for UnpricedReason {
    fn from(error: market_gifts::ModelLookupError) -> Self {
        use market_gifts::ModelLookupError;
        match error {
            ModelLookupError::UnknownCollection => Self::UnknownCollection,
            ModelLookupError::AmbiguousCollection => Self::AmbiguousCollection,
            ModelLookupError::UnknownModel => Self::UnknownModel,
            ModelLookupError::AmbiguousModel => Self::AmbiguousModel,
        }
    }
}
