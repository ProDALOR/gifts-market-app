//! The result and evidence produced by gift_floor, independent of transport presentation.
use crate::{FloorEvidence, Multiplier, ValuationMetric};
use serde::{Deserialize, Serialize};
pub const MINIMUM_MULTIPLIER_GIFTS: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftFloorEvidence {
    pub collection: FloorEvidence,
    pub model: FloorEvidence,
    pub backdrop: FloorEvidence,
    pub model_backdrop: FloorEvidence,
    pub minimum_multiplier_gifts: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftFloorValuation {
    pub collection_floor: ValuationMetric,
    pub model_floor: ValuationMetric,
    pub backdrop_floor: ValuationMetric,
    /// Lowest eligible ask matching both the model and backdrop.
    pub model_backdrop_floor: ValuationMetric,
    /// Dimensionless model floor / collection floor, rounded to two decimal places.
    pub model_multiplier: ValuationMetric<Multiplier>,
    /// Dimensionless backdrop floor / collection floor, rounded to two decimal places.
    pub backdrop_multiplier: ValuationMetric<Multiplier>,
    /// min(combined floor, collection floor * full-precision multipliers), then rounded to two decimals.
    pub final_mark: ValuationMetric,
    pub evidence: GiftFloorEvidence,
}
