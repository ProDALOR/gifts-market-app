//! Typed strategy arguments and their discoverable input requirements.
use super::{GiftAttributes, ResolvedTarget, ValuationTargetKind};
use crate::ValuationError;
use jiff::Timestamp;
use market_gifts::Period;
use serde::Serialize;
use std::sync::Arc;

/// Input requirements after resolving the selected valuation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum InputContract {
    Catalog,
    Selection,
    SelectionWithExecutionPeriod,
    GiftAttributes,
}
const SELECTION_TARGETS: &[ValuationTargetKind] = &[
    ValuationTargetKind::Selection,
    ValuationTargetKind::Gift,
    ValuationTargetKind::GiftAttributes,
];
impl InputContract {
    pub const fn targets(self) -> &'static [ValuationTargetKind] {
        match self {
            Self::Catalog => &[ValuationTargetKind::Catalog],
            Self::GiftAttributes => &[
                ValuationTargetKind::Gift,
                ValuationTargetKind::GiftAttributes,
            ],
            Self::Selection | Self::SelectionWithExecutionPeriod => SELECTION_TARGETS,
        }
    }
    pub const fn requires_period(self) -> bool {
        matches!(self, Self::SelectionWithExecutionPeriod)
    }
    /// Request selection convenience, not validation of a target's resolved facts.
    pub fn accepts(self, target: ValuationTargetKind, has_period: bool) -> bool {
        self.targets().contains(&target) && (has_period || !self.requires_period())
    }
}
/// Converts resolved domain facts and a previously checked period into typed arguments.
pub trait StrategyInput: Sized {
    const CONTRACT: InputContract;
    fn resolve(
        target: Arc<ResolvedTarget>,
        period: Option<ExecutionPeriod>,
    ) -> Result<Self, ValuationError>;
}
/// One resolved market selection; copies share the target facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionInput {
    target: Arc<ResolvedTarget>,
}
impl SelectionInput {
    pub fn target(&self) -> &ResolvedTarget {
        &self.target
    }
}
impl StrategyInput for SelectionInput {
    const CONTRACT: InputContract = InputContract::Selection;
    fn resolve(
        target: Arc<ResolvedTarget>,
        _period: Option<ExecutionPeriod>,
    ) -> Result<Self, ValuationError> {
        if !Self::CONTRACT.targets().contains(&target.kind()) {
            return Err(ValuationError::unsupported_target(
                "this strategy requires one market selection",
            ));
        }
        Ok(Self { target })
    }
}
/// A checked, nonempty, half-open historical period. Both bounds are always present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionPeriod {
    from: Timestamp,
    until: Timestamp,
}
impl ExecutionPeriod {
    pub fn new(from: Timestamp, until: Timestamp) -> Result<Self, ValuationError> {
        if from >= until {
            return Err(ValuationError::invalid(
                "execution period must have from before until",
            ));
        }
        if until > Timestamp::now() {
            return Err(ValuationError::invalid(
                "execution period cannot end in the future",
            ));
        }
        Ok(Self { from, until })
    }
    pub(crate) fn resolve(period: Option<&Period>) -> Result<Self, ValuationError> {
        let Some((from, until)) = period.and_then(|period| period.from.zip(period.until)) else {
            return Err(ValuationError::invalid(
                "execution strategies require explicit from and until bounds",
            ));
        };
        Self::new(from, until)
    }
    pub fn from(&self) -> Timestamp {
        self.from
    }
    pub fn until(&self) -> Timestamp {
        self.until
    }
    pub fn as_period(&self) -> Period {
        Period {
            from: Some(self.from),
            until: Some(self.until),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionInput {
    selection: SelectionInput,
    period: ExecutionPeriod,
}
impl ExecutionInput {
    pub fn selection(&self) -> &SelectionInput {
        &self.selection
    }
    pub fn period(&self) -> ExecutionPeriod {
        self.period
    }
}
impl StrategyInput for ExecutionInput {
    const CONTRACT: InputContract = InputContract::SelectionWithExecutionPeriod;
    fn resolve(
        target: Arc<ResolvedTarget>,
        period: Option<ExecutionPeriod>,
    ) -> Result<Self, ValuationError> {
        Ok(Self {
            selection: SelectionInput::resolve(target, period)?,
            period: period.ok_or_else(|| {
                ValuationError::invalid("execution strategies require an execution period")
            })?,
        })
    }
}
impl StrategyInput for GiftAttributes {
    const CONTRACT: InputContract = InputContract::GiftAttributes;
    fn resolve(
        target: Arc<ResolvedTarget>,
        _period: Option<ExecutionPeriod>,
    ) -> Result<Self, ValuationError> {
        match target.as_ref() {
            ResolvedTarget::GiftAttributes(attributes) => Ok(attributes.clone()),
            ResolvedTarget::Gift { gift, .. } => Ok(Self {
                collection_id: gift.collection_id.clone(),
                traits: gift.traits.clone(),
                number: gift.number,
            }),
            _ => Err(ValuationError::unsupported_target(
                "this strategy requires gift attributes",
            )),
        }
    }
}

#[cfg(test)]
#[path = "inputs_tests.rs"]
mod tests;

#[derive(Debug, Clone)]
pub struct CatalogInput {
    selection: crate::CatalogSelection,
}
impl CatalogInput {
    pub fn selection(&self) -> &crate::CatalogSelection {
        &self.selection
    }
}
impl StrategyInput for CatalogInput {
    const CONTRACT: InputContract = InputContract::Catalog;
    fn resolve(
        target: Arc<ResolvedTarget>,
        _: Option<ExecutionPeriod>,
    ) -> Result<Self, ValuationError> {
        let ResolvedTarget::Catalog(selection) = target.as_ref() else {
            return Err(ValuationError::unsupported_target(
                "this strategy requires a catalog selection",
            ));
        };
        Ok(Self {
            selection: selection.clone(),
        })
    }
}
