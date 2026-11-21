//! One typed registration table drives invocation, input validation and discovery.
use crate::{
    AskFloor, CatalogFloor, ExecutionMean, ExecutionMedian, ExecutionPeriod, GiftFloor,
    InputContract, ResolvedTarget, StrategyInput, StrategyValuation, ValuationContext,
    ValuationError,
};
use std::sync::Arc;

/// An algorithm declares the argument contract it consumes and owns its result type.
/// ```compile_fail
/// use market_valuation::{AskFloor, ResolvedGiftInput, Strategy, ValuationContext};
/// fn wrong_input(input: &ResolvedGiftInput, context: &ValuationContext<'_>) {
///     let _ = AskFloor.evaluate(input, context);
/// }
/// ```
pub trait Strategy {
    type Input: StrategyInput;
    type Output;
    const ID: &'static str;
    const NAME: &'static str;
    const DESCRIPTION: &'static str;
    const DOCUMENTATION: &'static str;
    const DISPLAY: crate::DisplayDefinition;
    fn evaluate(
        &self,
        input: &Self::Input,
        context: &ValuationContext<'_>,
    ) -> Result<Self::Output, ValuationError>;
}
#[derive(Debug, Clone, Copy)]
pub struct StrategyMetadata {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Markdown with inline ($...$) and display ($$...$$) LaTeX formulas.
    pub documentation: &'static str,
    pub input: InputContract,
    pub display: crate::DisplayDefinition,
}
impl StrategyMetadata {
    const fn of<S: Strategy>() -> Self {
        Self {
            id: S::ID,
            name: S::NAME,
            description: S::DESCRIPTION,
            documentation: S::DOCUMENTATION,
            input: S::Input::CONTRACT,
            display: S::DISPLAY,
        }
    }
}
type Evaluate = fn(
    Arc<ResolvedTarget>,
    Option<ExecutionPeriod>,
    &ValuationContext<'_>,
) -> Result<StrategyValuation, ValuationError>;
pub(crate) struct Registration {
    metadata: StrategyMetadata,
    evaluate: Evaluate,
}
macro_rules! register {
    ($strategy:ident => $output:ident) => {
        Registration {
            metadata: StrategyMetadata::of::<$strategy>(),
            evaluate: |target, period, context| {
                let input = <$strategy as Strategy>::Input::resolve(target, period)?;
                $strategy
                    .evaluate(&input, context)
                    .map(StrategyValuation::$output)
            },
        }
    };
}
static REGISTRY: &[Registration] = &[
    register!(CatalogFloor=>CatalogFloor),
    register!(AskFloor=>AskFloor),
    register!(ExecutionMean=>ExecutionMean),
    register!(ExecutionMedian=>ExecutionMedian),
    register!(GiftFloor=>GiftFloor),
];
/// Portfolio-only library actors are not advertised as available query strategies.
pub fn strategies() -> impl ExactSizeIterator<Item = &'static StrategyMetadata> + Clone {
    REGISTRY.iter().map(|entry| &entry.metadata)
}
pub fn strategy(id: &str) -> Option<&'static StrategyMetadata> {
    registration(id).map(Registration::metadata)
}
/// Discover compatible strategies without choosing an implicit service default.
/// Execution strategies are included only when the caller supplies a period.
pub fn strategies_for(
    target: crate::ValuationTargetKind,
    has_period: bool,
) -> impl Iterator<Item = &'static StrategyMetadata> {
    strategies().filter(move |entry| entry.input.accepts(target, has_period))
}
pub(crate) fn registration(id: &str) -> Option<&'static Registration> {
    REGISTRY.iter().find(|entry| entry.metadata.id == id)
}
impl Registration {
    pub(crate) fn metadata(&self) -> &StrategyMetadata {
        &self.metadata
    }
    pub(crate) fn evaluate(
        &self,
        target: Arc<ResolvedTarget>,
        period: Option<ExecutionPeriod>,
        context: &ValuationContext<'_>,
    ) -> Result<StrategyValuation, ValuationError> {
        (self.evaluate)(target, period, context)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_selection_includes_all_compatible_inputs_and_requires_opt_in_to_periods() {
        use crate::ValuationTargetKind;
        for target in InputContract::Selection.targets() {
            for has_period in [false, true] {
                let selected = strategies_for(*target, has_period).collect::<Vec<_>>();
                for metadata in strategies() {
                    assert_eq!(
                        selected.iter().any(|entry| entry.id == metadata.id),
                        metadata.input.targets().contains(target)
                            && (has_period || !metadata.input.requires_period()),
                    );
                }
            }
        }
        assert!(
            strategies_for(ValuationTargetKind::Gift, false)
                .any(|metadata| metadata.input == InputContract::ResolvedGift)
        );
        assert!(
            strategies_for(ValuationTargetKind::Selection, true)
                .all(|metadata| metadata.input != InputContract::ResolvedGift)
        );
    }
    #[test]
    fn discovery_and_execution_share_unique_typed_registrations() {
        assert!(strategies().len() > 0);
        let mut ids = std::collections::BTreeSet::new();
        for entry in REGISTRY {
            let metadata = &entry.metadata;
            assert!(crate::StrategyId::new(metadata.id).is_ok());
            assert!(ids.insert(metadata.id));
            assert!(!metadata.name.trim().is_empty());
            assert!(!metadata.description.trim().is_empty());
            assert!(!metadata.documentation.trim().is_empty());
            assert!(std::ptr::eq(strategy(metadata.id).unwrap(), metadata));
        }
        assert!(strategy("unregistered").is_none());
        for (id, contract) in [
            (AskFloor::ID, <AskFloor as Strategy>::Input::CONTRACT),
            (
                ExecutionMean::ID,
                <ExecutionMean as Strategy>::Input::CONTRACT,
            ),
            (
                ExecutionMedian::ID,
                <ExecutionMedian as Strategy>::Input::CONTRACT,
            ),
            (GiftFloor::ID, <GiftFloor as Strategy>::Input::CONTRACT),
        ] {
            assert_eq!(strategy(id).unwrap().input, contract);
        }
    }
}
