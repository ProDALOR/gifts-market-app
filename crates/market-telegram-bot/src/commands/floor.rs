use super::{FloorQuery, Reply, render};
use market_gifts::valuation::{ModelPosition, ValuationRequest};
use market_valuation::ValuationHandle;
pub(super) async fn execute(market: &ValuationHandle, query: FloorQuery) -> Reply {
    match market
        .mark(ValuationRequest {
            strategy: query.strategy,
            positions: vec![ModelPosition {
                model: query.model,
                quantity: 1,
            }],
            currency_id: query.currency,
        })
        .await
    {
        Ok(value) => render::floor(&value),
        Err(error) => render::error_reply(error),
    }
}
