//! Descriptions of gift strategies exposed by this service. No transport URLs or result erasure.
pub struct StrategyMetadata {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Markdown with inline ($...$) and display ($$...$$) LaTeX formulas.
    pub documentation: &'static str,
    pub targets: &'static [crate::ValuationTargetKind],
    pub requires_period: bool,
}

const TARGETS: &[crate::ValuationTargetKind] = &[
    crate::ValuationTargetKind::Collection,
    crate::ValuationTargetKind::Model,
    crate::ValuationTargetKind::Backdrop,
    crate::ValuationTargetKind::ModelBackdrop,
    crate::ValuationTargetKind::Gift,
];
pub const ASK_FLOOR: StrategyMetadata = StrategyMetadata {
    id: "ask_floor",
    name: "Ask floor",
    description: "Lowest eligible ask for a collection, trait selection or individual gift.",
    documentation: include_str!("market/ask_floor.md"),
    targets: TARGETS,
    requires_period: false,
};
pub const EXECUTION_MEAN: StrategyMetadata = StrategyMetadata {
    id: "execution_mean",
    name: "Execution mean",
    description: "Unweighted arithmetic mean of observed per-gift execution prices in an explicit period.",
    documentation: include_str!("market/executions.md"),
    targets: TARGETS,
    requires_period: true,
};
pub const EXECUTION_MEDIAN: StrategyMetadata = StrategyMetadata {
    id: "execution_median",
    name: "Execution median",
    description: "Median observed per-gift execution price in an explicit period.",
    documentation: include_str!("market/executions.md"),
    targets: TARGETS,
    requires_period: true,
};

/// Public gift-valuation strategies; portfolio-only library actors are not API services.
pub fn strategies() -> &'static [StrategyMetadata] {
    &[
        ASK_FLOOR,
        EXECUTION_MEAN,
        EXECUTION_MEDIAN,
        crate::gift_floor::METADATA,
    ]
}
pub fn strategy(id: &str) -> Option<&'static StrategyMetadata> {
    strategies().iter().find(|metadata| metadata.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discoverable_strategies_have_unique_valid_ids_and_own_their_documentation() {
        assert!(!strategies().is_empty());
        let mut ids = std::collections::BTreeSet::new();
        for metadata in strategies() {
            assert!(crate::StrategyId::new(metadata.id).is_ok());
            assert!(ids.insert(metadata.id));
            assert!(!metadata.name.trim().is_empty());
            assert!(!metadata.description.trim().is_empty());
            assert!(!metadata.documentation.trim().is_empty());
            assert!(std::ptr::eq(strategy(metadata.id).unwrap(), metadata));
        }
        assert!(strategy("unregistered").is_none());
    }
}
