//! Gift valuation actors, calculations, typed clients and lifecycle ownership.
mod display;
pub use display::{DisplayField, DisplayValue, ValuationDisplay};
mod calculation;
mod floors;
pub mod gift_floor;
pub use gift_floor::{
    GiftFloorActor, GiftFloorEvidence, GiftFloorMark, GiftFloorValuation,
    MINIMUM_MULTIPLIER_SAMPLES, ValuationHandle,
};
mod strategy;
pub use strategy::{StrategyMetadata, strategies, strategy};
mod client;
mod portfolio;
mod runtime;
pub use floors::FloorActor;
pub use portfolio::*;
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
