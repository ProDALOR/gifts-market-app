use super::{FloorQuery, Reply, render};
use market_actors::MarketDataHandle;
use market_gifts::valuation::{ModelPosition, ValuationRequest};
pub(super) async fn execute(market: &MarketDataHandle, query: FloorQuery) -> Reply {
    match market
        .value_models(ValuationRequest {
            positions: vec![ModelPosition {
                model: query.model,
                quantity: 1,
            }],
            currency_id: query.currency,
        })
        .await
    {
        Ok(value) => render::floor(value),
        Err(error) => render::error_reply(error),
    }
}
