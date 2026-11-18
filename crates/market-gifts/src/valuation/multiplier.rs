//! Ask-floor ratios. These measurements are not portfolio price estimators.
use super::{ModelPosition, StrategyId};
use crate::{Amount, BackdropId, CollectionId, CurrencyId, InvalidValue, ModelId, SyncStatus};
use serde::{Deserialize, Serialize};

pub const MINIMUM_MULTIPLIER_SAMPLES: u32 = 3;

/// Positive dimensionless ratio, rounded to 12 decimal places using midpoint-to-even rounding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", schema(value_type = String))]
pub struct Multiplier(Amount);
impl Multiplier {
    pub fn from_floors(trait_floor: &Amount, baseline: &Amount) -> Result<Self, InvalidValue> {
        if baseline.decimal() <= rust_decimal::Decimal::ZERO {
            return Err(InvalidValue("multiplier baseline must be positive"));
        }
        let ratio = trait_floor
            .decimal()
            .checked_div(baseline.decimal())
            .ok_or(InvalidValue("multiplier ratio overflow"))?
            .round_dp(12);
        Self::try_from(ratio.to_string())
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
impl TryFrom<String> for Multiplier {
    type Error = InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let amount = Amount::new(value)?;
        if amount.decimal() <= rust_decimal::Decimal::ZERO {
            return Err(InvalidValue("multiplier must be positive"));
        }
        Ok(Self(amount))
    }
}
impl From<Multiplier> for String {
    fn from(value: Multiplier) -> Self {
        value.0.to_string()
    }
}
impl std::fmt::Display for Multiplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ModelMultiplierRequest {
    pub collection_id: CollectionId,
    pub model_id: ModelId,
    pub currency_id: CurrencyId,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct BackdropMultiplierRequest {
    pub collection_id: CollectionId,
    pub backdrop_id: BackdropId,
    pub currency_id: CurrencyId,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AskEvidence {
    pub floor: Amount,
    /// Number of eligible listing rows, not total gift quantity or independent sellers.
    pub samples: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct MultiplierEvidence {
    pub collection: AskEvidence,
    pub selected_trait: AskEvidence,
    pub minimum_samples: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ModelMultiplier {
    pub collection_id: CollectionId,
    pub model_id: ModelId,
    pub currency_id: CurrencyId,
    pub factor: Multiplier,
    pub evidence: MultiplierEvidence,
    pub sync: SyncStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct BackdropMultiplier {
    pub collection_id: CollectionId,
    pub backdrop_id: BackdropId,
    pub currency_id: CurrencyId,
    pub factor: Multiplier,
    pub evidence: MultiplierEvidence,
    pub sync: SyncStatus,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PortfolioPositions {
    pub positions: Vec<ModelPosition>,
    pub currency_id: CurrencyId,
}
/// Every strategy owns its request contract. Selection is always explicit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationQuery {
    ModelFloor(PortfolioPositions),
    CollectionFloor(PortfolioPositions),
    ModelMultiplier(ModelMultiplierRequest),
    BackdropMultiplier(BackdropMultiplierRequest),
}
impl StrategyId {
    pub fn model_multiplier() -> Self {
        Self("model_multiplier".into())
    }
    pub fn backdrop_multiplier() -> Self {
        Self("backdrop_multiplier".into())
    }
    pub fn is_portfolio_strategy(&self) -> bool {
        matches!(self.as_str(), "model_floor" | "collection_floor")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationErrorContext {
    Portfolio {
        unpriced: Vec<super::ModelValue>,
    },
    Multiplier {
        collection_id: CollectionId,
        selected_trait: ValuationTrait,
        collection_floor: Option<Amount>,
        trait_floor: Option<Amount>,
        collection_samples: u32,
        trait_samples: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationTrait {
    Model(ModelId),
    Backdrop(BackdropId),
}
