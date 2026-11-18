//! Descriptions of gift strategies exposed by this service. No transport URLs or result erasure.
pub struct StrategyMetadata {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Markdown with inline ($...$) and display ($$...$$) LaTeX formulas.
    pub documentation: &'static str,
}

/// Public gift-valuation strategies; portfolio-only library actors are not API services.
pub fn strategies() -> &'static [StrategyMetadata] {
    &[crate::gift_floor::METADATA]
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
