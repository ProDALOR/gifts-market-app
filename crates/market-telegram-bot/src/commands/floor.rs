use super::{FloorQuery, Reply, render};
use market_actors::valuation::ValuationHandle;
use market_gifts::valuation::{ModelPosition, ValuationRequest};
pub(super) async fn execute(market: &ValuationHandle, query: FloorQuery) -> Reply {
    match market
        .mark(ValuationRequest {
            positions: vec![ModelPosition {
                model: query.model,
                quantity: 1,
            }],
            currency_id: query.currency,
        })
        .await
    {
        Ok(value) => render::floor(&value),
        Err(error) => render::valuation_error(error),
    }
}
