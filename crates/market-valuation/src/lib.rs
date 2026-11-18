//! Gift valuation actors, calculations, typed clients and lifecycle ownership.
mod calculation;
mod floors;
mod gift;
mod gift_calculation;
pub use gift::*;
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
