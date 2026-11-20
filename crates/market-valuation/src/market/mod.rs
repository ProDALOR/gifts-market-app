//! Independently queryable targets and typed strategies, evaluated in one admitted snapshot.
mod actor;
mod context;
mod execution;
mod inputs;
mod read;
pub(crate) use read::ValuationRead;
mod strategies;
mod target;
use crate::*;
pub(crate) use actor::MarketValuationActor;
pub use context::ValuationContext;
pub use inputs::{
    CatalogInput, ExecutionInput, ExecutionPeriod, InputContract, ResolvedGiftInput,
    SelectionInput, StrategyInput,
};
use market_gifts::{Amount, CurrencyId, Period};
use serde::{Deserialize, Serialize};
pub use strategies::{AskFloor, ExecutionMean, ExecutionMedian};
pub use target::{
    CatalogDetails, CatalogSelection, PairSelection, ResolvedTarget, ValuationTarget,
    ValuationTargetKind,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationQuery {
    pub target: ValuationTarget,
    pub currency_id: CurrencyId,
    /// Explicit, nonempty selection from the strategy catalog. Input order is preserved.
    #[cfg_attr(feature = "schema", schema(min_items = 1, max_items = 8))]
    pub strategies: Vec<StrategyId>,
    /// Execution strategies require a nonempty historical period with both bounds; until is exclusive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}
impl ValuationQuery {
    /// Checks request options without reading the market. Target and currency facts
    /// are checked against the admitted snapshot by `ValuationHandle::query`.
    pub fn validate(&self) -> Result<(), ValuationError> {
        self.resolve_options().map(|_| ())
    }
    pub(crate) fn prepare(self) -> Result<PreparedQuery, ValuationError> {
        let (strategies, period) = self.resolve_options()?;
        Ok(PreparedQuery {
            target: self.target,
            currency_id: self.currency_id,
            strategies,
            period,
        })
    }
    fn resolve_options(
        &self,
    ) -> Result<
        (
            Vec<&'static crate::strategy::Registration>,
            Option<ExecutionPeriod>,
        ),
        ValuationError,
    > {
        if let ValuationTarget::Catalog(selection) = &self.target {
            selection.validate()?;
        }
        if self.strategies.is_empty() || self.strategies.len() > 8 {
            return Err(ValuationError::invalid("select 1..=8 valuation strategies"));
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut strategies = Vec::with_capacity(self.strategies.len());
        for id in &self.strategies {
            let registration = crate::strategy::registration(id.as_str())
                .ok_or_else(|| ValuationError::invalid("unknown valuation strategy"))?;
            if !seen.insert(id) {
                return Err(ValuationError::invalid("repeated valuation strategy"));
            }
            strategies.push(registration);
        }
        let period = if strategies
            .iter()
            .any(|entry| entry.metadata().input.requires_period())
        {
            Some(ExecutionPeriod::resolve(self.period.as_ref())?)
        } else {
            if let Some(period) = &self.period {
                period
                    .validate()
                    .map_err(|error| ValuationError::invalid(error.to_string()))?;
            }
            None
        };
        Ok((strategies, period))
    }
}
/// Immutable request options checked before admission; target facts still resolve in the pinned view.
#[derive(Clone)]
pub(crate) struct PreparedQuery {
    target: ValuationTarget,
    currency_id: CurrencyId,
    strategies: Vec<&'static crate::strategy::Registration>,
    period: Option<ExecutionPeriod>,
}
impl PreparedQuery {
    pub(crate) fn currency(&self) -> &CurrencyId {
        &self.currency_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AskFloorValuation {
    pub price: ValuationMetric,
    pub evidence: FloorEvidence,
}
/// Shared current-ask evidence, independent of any individual-gift strategy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct FloorEvidence {
    pub listings: u32,
    /// Distinct identifiable collectibles; cross-listings count once and broad criteria do not count.
    pub gifts: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ExecutionEvidence {
    pub executions: u32,
    pub gifts: u32,
    pub venues: u32,
    pub minimum_executions: u32,
    pub minimum_gifts: u32,
    #[cfg_attr(feature="schema",schema(value_type=Option<String>,format=DateTime))]
    pub first_execution: Option<jiff::Timestamp>,
    #[cfg_attr(feature="schema",schema(value_type=Option<String>,format=DateTime))]
    pub last_execution: Option<jiff::Timestamp>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ExecutionPriceValuation {
    pub price: ValuationMetric,
    pub period: Period,
    /// Observed records only: this does not assert complete history or independent sellers.
    pub evidence: ExecutionEvidence,
}
impl ExecutionEvidence {
    fn summary(&self) -> String {
        let mut text = format!(
            "{} executions · {} gifts · {} venues",
            self.executions, self.gifts, self.venues
        );
        if let Some((first, last)) = self.first_execution.zip(self.last_execution) {
            text.push_str(&format!(
                " · observed {} – {} UTC",
                first.strftime("%Y-%m-%d %H:%M"),
                last.strftime("%Y-%m-%d %H:%M")
            ));
        }
        text
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "strategy", content = "data", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum StrategyValuation {
    CatalogFloor(std::sync::Arc<crate::CatalogFloorValuation>),
    AskFloor(AskFloorValuation),
    ExecutionMean(ExecutionPriceValuation),
    ExecutionMedian(ExecutionPriceValuation),
    GiftFloor(GiftFloorValuation),
}
impl StrategyValuation {
    pub fn strategy(&self) -> &'static str {
        match self {
            Self::CatalogFloor(_) => CatalogFloor::ID,
            Self::AskFloor(_) => AskFloor::ID,
            Self::ExecutionMean(_) => ExecutionMean::ID,
            Self::ExecutionMedian(_) => ExecutionMedian::ID,
            Self::GiftFloor(_) => GiftFloor::ID,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct StrategyFailure {
    pub code: ValuationErrorCode,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum StrategyOutcome {
    Value(StrategyValuation),
    Error {
        strategy: StrategyId,
        error: StrategyFailure,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationReport {
    pub target: ResolvedTarget,
    pub currency_id: CurrencyId,
    #[cfg_attr(feature="schema",schema(value_type=String,format=DateTime))]
    pub evaluated_at: jiff::Timestamp,
    pub stale: bool,
    pub results: Vec<StrategyOutcome>,
}
impl StrategyOutcome {
    pub fn strategy(&self) -> &str {
        match self {
            Self::Value(value) => value.strategy(),
            Self::Error { strategy, .. } => strategy.as_str(),
        }
    }
    pub fn display<'a>(&'a self, currency: &'a CurrencyId) -> ValuationDisplay<'a> {
        let (price, summary, note) = match self {
            Self::Value(StrategyValuation::CatalogFloor(data)) => {
                return ValuationDisplay {
                    fields: vec![],
                    summary: Some(format!("{} collections", data.collections.len())),
                    note: "Current eligible asks, before fees.",
                };
            }
            Self::Value(StrategyValuation::GiftFloor(data)) => return data.display(currency),
            Self::Value(StrategyValuation::AskFloor(data)) => (
                &data.price,
                Some(format!(
                    "{} gifts · {} listings",
                    data.evidence.gifts, data.evidence.listings
                )),
                "Lowest eligible ask, before fees.",
            ),
            Self::Value(StrategyValuation::ExecutionMean(data)) => (
                &data.price,
                Some(data.evidence.summary()),
                "Unweighted arithmetic mean per execution record; observed history only.",
            ),
            Self::Value(StrategyValuation::ExecutionMedian(data)) => (
                &data.price,
                Some(data.evidence.summary()),
                "Median per execution record; observed history only.",
            ),
            Self::Error { error, .. } => {
                return ValuationDisplay {
                    fields: vec![DisplayField {
                        id: "price",
                        label: "Value",
                        primary: true,
                        basis: false,
                        attribute: None,
                        value: DisplayValue::Unavailable {
                            label: "Unavailable",
                            message: &error.message,
                            detail: None,
                        },
                    }],
                    summary: None,
                    note: "The other strategy results remain available.",
                };
            }
        };
        let decimals = matches!(
            self,
            Self::Value(
                StrategyValuation::ExecutionMean(_) | StrategyValuation::ExecutionMedian(_)
            )
        )
        .then_some(2);
        ValuationDisplay {
            fields: vec![
                DisplayField::currency("price", "Value", price, currency, decimals).primary(),
            ],
            summary,
            note,
        }
    }
}
pub(crate) fn unavailable(
    code: ValuationIssueCode,
    message: impl Into<String>,
    required: Option<u32>,
    available: Option<u32>,
) -> ValuationMetric<Amount> {
    ValuationMetric::Unavailable {
        error: ValuationIssue {
            code,
            message: message.into(),
            required_samples: required,
            available_samples: available,
            dependency: None,
        },
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod watch_tests;
