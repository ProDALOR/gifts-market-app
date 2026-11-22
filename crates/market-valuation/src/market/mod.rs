//! Independently queryable targets and typed strategies, evaluated in one admitted snapshot.
mod actor;
mod context;
mod execution;
mod inputs;
mod read;
mod read_cache;
mod read_set;
pub(crate) use read::ValuationRead;
mod strategies;
mod target;
use crate::*;
pub(crate) use actor::MarketValuationActor;
pub use context::ValuationContext;
pub use inputs::{
    CatalogInput, ExecutionInput, ExecutionPeriod, InputContract, SelectionInput, StrategyInput,
};
use market_gifts::{Amount, CurrencyId, Period};
use serde::{Deserialize, Serialize};
pub use strategies::{AskFloor, ExecutionMean, ExecutionMedian};
pub use target::{
    CatalogDetails, CatalogSelection, GiftAttributes, PairSelection, ResolvedTarget,
    ValuationTarget, ValuationTargetKind,
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
        self.clone().prepare().map(|_| ())
    }
    pub(crate) fn prepare(mut self) -> Result<PreparedQuery, ValuationError> {
        self.target.normalize()?;
        let (strategies, period) = self.resolve_options()?;
        Ok(PreparedQuery {
            target: self.target,
            currency_id: self.currency_id,
            strategies,
            period,
            external_gift: None,
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
            if self.period.is_some() {
                return Err(ValuationError::invalid(
                    "period is not supported by the selected strategies",
                ));
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
    external_gift: Option<market_gifts::TelegramGiftDetails>,
}
impl PreparedQuery {
    fn same_query(&self, other: &Self) -> bool {
        self.target == other.target
            && self.external_gift == other.external_gift
            && self.currency_id == other.currency_id
            && self.period == other.period
            && self.strategies.len() == other.strategies.len()
            && self
                .strategies
                .iter()
                .zip(&other.strategies)
                .all(|(left, right)| std::ptr::eq(*left, *right))
    }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "strategy", content = "data", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum StrategyValuation {
    CatalogFloor(std::sync::Arc<crate::CatalogFloorValuation>),
    AskFloor(FloorValuation),
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
/// Public computation failures; storage retries and actor identities stay internal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum StrategyFailureCode {
    InvalidQuery,
    NotFound,
    CapacityExceeded,
    Unavailable,
    TimedOut,
    NotEnoughData,
    InvalidBaseline,
    UnsupportedTarget,
}
impl From<ValuationErrorCode> for StrategyFailureCode {
    fn from(code: ValuationErrorCode) -> Self {
        match code {
            ValuationErrorCode::InvalidQuery | ValuationErrorCode::UnsupportedStrategy => {
                Self::InvalidQuery
            }
            ValuationErrorCode::RestartRead | ValuationErrorCode::Unavailable => Self::Unavailable,
            ValuationErrorCode::NotFound => Self::NotFound,
            ValuationErrorCode::CapacityExceeded => Self::CapacityExceeded,
            ValuationErrorCode::TimedOut => Self::TimedOut,
            ValuationErrorCode::NotEnoughData => Self::NotEnoughData,
            ValuationErrorCode::InvalidBaseline => Self::InvalidBaseline,
            ValuationErrorCode::UnsupportedTarget => Self::UnsupportedTarget,
        }
    }
}
impl From<ValuationError> for StrategyFailure {
    fn from(error: ValuationError) -> Self {
        let code = StrategyFailureCode::from(error.code());
        let message = if code == StrategyFailureCode::Unavailable {
            tracing::debug!(
                reason = error.message(),
                "valuation computation unavailable"
            );
            "Valuation data is temporarily unavailable".into()
        } else {
            error.message().to_owned()
        };
        Self { code, message }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct StrategyFailure {
    pub code: StrategyFailureCode,
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
    pub fn display(&self, currency: &CurrencyId) -> ValuationDisplay {
        const FALLBACK: crate::DisplayDefinition = crate::DisplayDefinition {
            fields: &[crate::FieldDefinition::currency("price", "Value", None).primary()],
            evidence: &[],
            note: "",
        };
        let display =
            crate::strategy(self.strategy()).map_or(FALLBACK, |metadata| metadata.display);
        match self {
            Self::Value(value) => {
                let value = serde_json::to_value(value).expect("typed valuation serializes");
                display.render(&value["data"], None, currency)
            }
            Self::Error { error, .. } => {
                display.render(&serde_json::Value::Null, Some(&error.message), currency)
            }
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

#[cfg(test)]
mod slug_tests;
