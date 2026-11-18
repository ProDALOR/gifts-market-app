//! One public report with one snapshot revision and evaluation time.
use serde::Serialize;
use utoipa::ToSchema;
#[derive(Debug, Serialize, ToSchema)]
pub struct GiftValuationResponse<'a> {
    pub strategy: &'a str,
    pub revision: &'a str,
    #[schema(value_type = String, format = DateTime)]
    pub evaluated_at: jiff::Timestamp,
    #[serde(flatten)]
    pub data: &'a market_valuation::GiftFloorValuation,
}
impl<'a> From<&'a market_valuation::GiftFloorMark> for GiftValuationResponse<'a> {
    fn from(value: &'a market_valuation::GiftFloorMark) -> Self {
        Self {
            strategy: value.strategy,
            revision: &value.revision,
            evaluated_at: value.evaluated_at,
            data: &value.data,
        }
    }
}
