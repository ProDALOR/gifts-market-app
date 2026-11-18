//! One coherent gift report. Floors are prices; multipliers are dimensionless.
use market_gifts::{Amount, CollectibleGift, CurrencyId, GiftSlug, SyncStatus};
use serde::{Deserialize, Serialize};

pub const MINIMUM_MULTIPLIER_SAMPLES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationIssueCode {
    NotEnoughData,
    MissingTrait,
    InvalidBaseline,
    CalculationOverflow,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationIssue {
    pub code: ValuationIssueCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_samples: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_samples: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationMetric {
    Value { value: Amount },
    Unavailable { error: ValuationIssue },
}
impl ValuationMetric {
    pub fn value(&self) -> Option<&Amount> {
        match self {
            Self::Value { value } => Some(value),
            Self::Unavailable { .. } => None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftValuationEvidence {
    /// Counts are eligible listing rows, not independent gifts or sellers.
    pub collection_samples: u32,
    pub model_samples: u32,
    pub backdrop_samples: u32,
    pub model_backdrop_samples: u32,
    pub minimum_multiplier_samples: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftValuation {
    pub slug: Option<GiftSlug>,
    pub gift: CollectibleGift,
    pub currency_id: CurrencyId,
    pub collection_floor: ValuationMetric,
    pub model_floor: ValuationMetric,
    pub backdrop_floor: ValuationMetric,
    /// Lowest eligible ask matching both the model and backdrop.
    pub model_backdrop_floor: ValuationMetric,
    /// Dimensionless model floor / collection floor, rounded to two decimal places.
    pub model_multiplier: ValuationMetric,
    /// Dimensionless backdrop floor / collection floor, rounded to two decimal places.
    pub backdrop_multiplier: ValuationMetric,
    /// min(combined floor, collection floor * full-precision multipliers), then rounded to two decimals.
    pub final_mark: ValuationMetric,
    pub evidence: GiftValuationEvidence,
    pub sync: SyncStatus,
}
