//! Individual-gift estimate from the shared current-ask statistics.
mod calculation;
mod display;
mod result;
pub use result::*;
pub const METADATA: crate::StrategyMetadata = crate::StrategyMetadata {
    id: "gift_floor",
    name: "Gift floor",
    description: "A floor-based heuristic using collection and trait floors, capped by the matching model-and-backdrop floor.",
    documentation: include_str!("methodology.md"),
    targets: &[crate::ValuationTargetKind::Gift],
    requires_period: false,
};
#[cfg(test)]
pub(crate) mod tests;
