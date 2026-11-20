//! Independently queryable targets and typed estimators, evaluated in one admitted snapshot.
mod actor;
mod estimators;
mod execution;
mod target;
use crate::*;
pub(crate) use actor::MarketValuationActor;
pub use estimators::{AskFloor, Estimator, ExecutionMean, ExecutionMedian, ValuationContext};
use market_gifts::{Amount, CurrencyId, Period};
use serde::{Deserialize, Serialize};
pub use target::{ResolvedTarget, ValuationTarget, ValuationTargetKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationQuery {
    pub target: ValuationTarget,
    pub currency_id: CurrencyId,
    /// Explicit, nonempty selection from the strategy catalog. Input order is preserved.
    #[cfg_attr(feature = "schema", schema(min_items = 1, max_items = 8))]
    pub strategies: Vec<StrategyId>,
    /// Both bounds are required when selecting an execution strategy; until is exclusive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}
impl ValuationQuery {
    pub fn validate(&self) -> Result<(), ValuationError> {
        if self.strategies.is_empty() || self.strategies.len() > 8 {
            return Err(ValuationError::invalid("select 1..=8 valuation strategies"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for id in &self.strategies {
            if crate::strategy(id.as_str()).is_none() || !seen.insert(id) {
                return Err(ValuationError::invalid(
                    "unknown or repeated valuation strategy",
                ));
            }
        }
        if let Some(period) = &self.period {
            period
                .validate()
                .map_err(|e| ValuationError::invalid(e.to_string()))?;
        }
        if self
            .strategies
            .iter()
            .any(|id| crate::strategy(id.as_str()).is_some_and(|metadata| metadata.requires_period))
        {
            if !self
                .period
                .as_ref()
                .is_some_and(|p| p.from.is_some() && p.until.is_some())
            {
                return Err(ValuationError::invalid(
                    "execution strategies require explicit from and until bounds",
                ));
            }
            if self.period.as_ref().unwrap().until.unwrap() > jiff::Timestamp::now() {
                return Err(ValuationError::invalid(
                    "execution period cannot end in the future",
                ));
            }
        }
        Ok(())
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
    AskFloor(AskFloorValuation),
    ExecutionMean(ExecutionPriceValuation),
    ExecutionMedian(ExecutionPriceValuation),
    GiftFloor(GiftFloorValuation),
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
    pub fn display<'a>(&'a self, currency: &'a CurrencyId) -> ValuationDisplay<'a> {
        let (price, summary, note) = match self {
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
