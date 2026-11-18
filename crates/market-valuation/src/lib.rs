//! Gift valuation actors, calculations, typed clients and lifecycle ownership.
mod calculation;
mod floors;
mod multipliers;
mod registry;
mod runtime;
pub use floors::FloorActor;
pub use multipliers::{BackdropMultiplierActor, ModelMultiplierActor};
pub use registry::*;
pub use runtime::*;

#[cfg(test)]
mod integration_tests;

mod error;
pub use error::{ValuationError, ValuationErrorCode};
