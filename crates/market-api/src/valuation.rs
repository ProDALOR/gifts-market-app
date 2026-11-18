//! Borrowed HTTP presentation of a shared valuation mark; scheduling stays in market-valuation.
use market_gifts::valuation::Valuation;
use market_valuation::Mark;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ValuationCoverage {
    /// Collectibles with a quote from the selected strategy in the requested currency.
    pub priced: u32,
    /// Collectibles excluded from the estimate because the selected strategy had no eligible quote.
    pub unpriced: u32,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct ValuationMark<'a> {
    #[schema(value_type=market_gifts::valuation::StrategyId)]
    pub strategy: &'a str,
    /// Exact valuation, quote currency, model rows and source synchronization status.
    pub value: &'a Valuation,
    /// Source snapshot revision used by every row in this mark.
    pub revision: &'a str,
    /// UTC evaluation time; cache hits retain the original timestamp.
    #[schema(value_type = String, format = DateTime)]
    pub evaluated_at: jiff::Timestamp,
    pub coverage: ValuationCoverage,
}
impl<'a> From<&'a Mark<Valuation>> for ValuationMark<'a> {
    fn from(mark: &'a Mark<Valuation>) -> Self {
        Self {
            strategy: mark.strategy,
            value: &mark.value,
            revision: &mark.revision,
            evaluated_at: mark.evaluated_at,
            coverage: ValuationCoverage {
                priced: mark.coverage.priced,
                unpriced: mark.coverage.unpriced,
            },
        }
    }
}
