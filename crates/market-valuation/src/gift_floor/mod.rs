//! Individual-gift estimate from the shared current-ask statistics.
mod calculation;
mod display;
mod result;
pub use result::*;
mod strategy;
pub use strategy::GiftFloor;
#[cfg(test)]
pub(crate) mod tests;
