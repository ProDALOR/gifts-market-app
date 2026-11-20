//! Gift valuation actors, calculations, typed clients and lifecycle ownership.
mod display;
pub use display::{DisplayAttribute, DisplayField, DisplayValue, ValuationDisplay};
pub mod gift_floor;
pub use gift_floor::{
    FloorEvidence, GiftFloorActor, GiftFloorEvidence, GiftFloorMark, GiftFloorValuation,
    MINIMUM_MULTIPLIER_GIFTS, ValuationHandle,
};
mod strategy;
pub use strategy::{StrategyMetadata, strategies, strategy};
mod client;
#[cfg(feature = "portfolio")]
mod portfolio;
mod runtime;
#[cfg(feature = "portfolio")]
pub use portfolio::{
    FloorActor, FloorStrategyHandle, ModelPosition, ModelValue, PortfolioPositions,
    PortfolioValuationHandle, UnpricedReason, Valuation, ValuationErrorContext, ValuationRequest,
};
pub use runtime::*;

#[cfg(test)]
mod integration_tests;

mod error;
pub use error::{ValuationError, ValuationErrorCode};

mod types;
pub use types::*;

pub mod source;
pub use source::{ValuationSnapshot, ValuationSource};
#[cfg(test)]
mod test_support;
