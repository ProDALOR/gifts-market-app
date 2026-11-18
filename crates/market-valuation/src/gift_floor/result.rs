//! The result and evidence produced by gift_floor, independent of transport presentation.
use crate::ValuationMetric;
use market_gifts::{CollectibleGift, CurrencyId, GiftSlug, SyncStatus};
use serde::{Deserialize, Serialize};
pub const MINIMUM_MULTIPLIER_SAMPLES: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftFloorEvidence {
    /// Counts are eligible listing rows, not independent gifts or sellers.
    pub collection_samples: u32,
    pub model_samples: u32,
    pub backdrop_samples: u32,
    pub model_backdrop_samples: u32,
    pub minimum_multiplier_samples: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftFloorValuation {
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
    pub evidence: GiftFloorEvidence,
    pub sync: SyncStatus,
}
