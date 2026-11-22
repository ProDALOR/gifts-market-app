//! The result produced by gift_floor, independent of transport presentation.
use crate::{FloorValuation, Multiplier, ValuationMetric};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftFloorBasis {
    Additive,
    PairCapped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftFloorValuation {
    pub collection_floor: FloorValuation,
    pub model_floor: FloorValuation,
    pub backdrop_floor: FloorValuation,
    /// Lowest eligible ask matching both the model and backdrop.
    pub model_backdrop_floor: FloorValuation,
    /// Positive premiums above the collection floor; full calculation precision.
    pub model_premium: ValuationMetric,
    pub backdrop_premium: ValuationMetric,
    /// Collection floor plus nonnegative model and backdrop premiums, before any pair cap.
    pub additive_estimate: ValuationMetric,
    /// Dimensionless model floor / collection floor, rounded to two decimal places.
    pub model_multiplier: ValuationMetric<Multiplier>,
    /// Dimensionless backdrop floor / collection floor, rounded to two decimal places.
    pub backdrop_multiplier: ValuationMetric<Multiplier>,
    /// Additive estimate, optionally capped by the matching pair floor, rounded to two decimals.
    pub final_mark: ValuationMetric,
    /// Absent when required floors are unavailable. PairCapped means matching pair evidence exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis: Option<GiftFloorBasis>,
}
