//! Borrowed HTTP marks; typed data and evidence remain owned by each strategy.
use market_gifts::valuation::{BackdropMultiplier, ModelMultiplier, Valuation};
use market_valuation::Mark;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ValuationMark<'a> {
    #[schema(value_type = market_gifts::valuation::StrategyId)]
    pub strategy: &'a str,
    pub data: &'a Valuation,
    pub revision: &'a str,
    #[schema(value_type = String, format = DateTime)]
    pub evaluated_at: jiff::Timestamp,
}
impl<'a> From<&'a Mark<Valuation>> for ValuationMark<'a> {
    fn from(mark: &'a Mark<Valuation>) -> Self {
        Self {
            strategy: mark.strategy,
            data: &mark.data,
            revision: &mark.revision,
            evaluated_at: mark.evaluated_at,
        }
    }
}
#[derive(Debug, Serialize, ToSchema)]
pub struct MarkData<'a, T> {
    pub data: &'a T,
    pub revision: &'a str,
    #[schema(value_type = String, format = DateTime)]
    pub evaluated_at: jiff::Timestamp,
}
impl<'a, T> From<&'a Mark<T>> for MarkData<'a, T> {
    fn from(mark: &'a Mark<T>) -> Self {
        Self {
            data: &mark.data,
            revision: &mark.revision,
            evaluated_at: mark.evaluated_at,
        }
    }
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum StrategyMark<'a> {
    ModelFloor(MarkData<'a, Valuation>),
    CollectionFloor(MarkData<'a, Valuation>),
    ModelMultiplier(MarkData<'a, ModelMultiplier>),
    BackdropMultiplier(MarkData<'a, BackdropMultiplier>),
}
impl<'a> From<&'a market_valuation::ValuationResult> for StrategyMark<'a> {
    fn from(result: &'a market_valuation::ValuationResult) -> Self {
        use market_valuation::ValuationResult as R;
        match result {
            R::ModelFloor(m) => Self::ModelFloor(m.as_ref().into()),
            R::CollectionFloor(m) => Self::CollectionFloor(m.as_ref().into()),
            R::ModelMultiplier(m) => Self::ModelMultiplier(m.as_ref().into()),
            R::BackdropMultiplier(m) => Self::BackdropMultiplier(m.as_ref().into()),
        }
    }
}
