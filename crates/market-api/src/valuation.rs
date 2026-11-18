//! Borrowed HTTP marks; typed data and evidence remain owned by each strategy.
use market_gifts::valuation::{BackdropMultiplier, ModelMultiplier, Valuation};
use market_valuation::Mark;
use serde::Serialize;
use utoipa::ToSchema;

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

#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum StrategyReport<'a> {
    Mark(StrategyMark<'a>),
    Error {
        strategy: &'a market_gifts::valuation::StrategyId,
        error: crate::error::ApiError,
    },
}
#[derive(Debug, Serialize, ToSchema)]
pub struct GiftValuationResponse<'a> {
    pub slug: Option<&'a market_gifts::GiftSlug>,
    pub gift: &'a market_gifts::CollectibleGift,
    pub currency_id: &'a market_gifts::CurrencyId,
    pub gift_revision: &'a str,
    pub strategies: Vec<StrategyReport<'a>>,
}
impl<'a> From<&'a market_valuation::GiftValuationResult> for GiftValuationResponse<'a> {
    fn from(value: &'a market_valuation::GiftValuationResult) -> Self {
        Self {
            slug: value.slug.as_ref(),
            gift: &value.gift,
            currency_id: &value.currency_id,
            gift_revision: &value.gift_revision,
            strategies: value
                .strategies
                .iter()
                .map(|row| match &row.result {
                    Ok(mark) => StrategyReport::Mark(mark.into()),
                    Err(error) => StrategyReport::Error {
                        strategy: &row.strategy,
                        error: error.clone().into(),
                    },
                })
                .collect(),
        }
    }
}
