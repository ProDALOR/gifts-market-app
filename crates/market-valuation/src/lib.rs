//! Gift valuation actors, calculations, typed clients and lifecycle ownership.
mod display;
pub use display::{
    DisplayDefinition, DisplayField, DisplayValue, EvidenceDefinition, FieldDefinition,
    FieldFormat, ValuationDisplay,
};
pub mod gift_floor;
pub use gift_floor::{GiftFloor, GiftFloorBasis, GiftFloorValuation};
mod strategy;
pub use strategy::{
    Strategy, StrategyMetadata, StrategyPurpose, strategies, strategies_for, strategy,
};
mod client;
mod market;
pub use market::*;
mod service;
pub use service::{ValuationHandle, ValuationWatch};
pub type ValuationTask = Task;
mod floor_projection;
mod floors;
pub use floor_projection::{
    CatalogFloor, CatalogFloorValuation, CollectionFloors, PairFloor, TraitFloors,
};
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

#[cfg(test)]
mod preparation_benchmark;

mod inventory;
pub use inventory::{InventoryValuation, InventoryValue};
