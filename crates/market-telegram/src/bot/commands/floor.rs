use super::{FloorQuery, Reply, render};
use market_actors::{
    MarketDataHandle,
    valuation::{ModelLookup, ModelPosition, ValuationRequest},
};
pub(super) async fn execute(market: &MarketDataHandle, query: FloorQuery) -> Reply {
    match market
        .value_models(ValuationRequest {
            positions: vec![ModelPosition {
                model: ModelLookup {
                    namespace: None,
                    collection: query.collection,
                    attribute_id: "tg.model".into(),
                    model: query.model,
                },
                quantity: 1,
            }],
            currency_id: query.currency.0,
        })
        .await
    {
        Ok(value) => render::floor(value),
        Err(error) => render::error_reply(error),
    }
}
