use super::{QuoteCurrency, Reply, render};
use crate::bot::{BotError, GiftSource, InventoryError, ProfileGift};
use market_actors::{
    MarketDataHandle,
    valuation::{ModelLookup, ModelPosition, ValuationRequest},
};
fn position(gift: &ProfileGift) -> ModelPosition {
    ModelPosition {
        model: ModelLookup {
            namespace: Some("tg".into()),
            collection: gift.collection_id.to_string(),
            attribute_id: "tg.model".into(),
            model: gift.model_document_id.to_string(),
        },
        quantity: 1,
    }
}
pub(super) async fn execute<S: GiftSource>(
    market: &MarketDataHandle,
    source: &S,
    currency: QuoteCurrency,
) -> Result<Reply, BotError> {
    let inventory = match source.inventory().await {
        Ok(inventory) => inventory,
        Err(InventoryError::Authorization) => return Err(BotError::Authorization),
        Err(error) => return Ok(render::inventory_error(error)),
    };
    Ok(
        match market
            .value_models(ValuationRequest {
                positions: inventory.gifts.iter().map(position).collect(),
                currency_id: currency.0,
            })
            .await
        {
            Ok(value) => render::portfolio(value, &inventory),
            Err(error) => render::error_reply(error),
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::GiftRelation;
    #[test]
    fn telegram_identifiers_become_explicit_catalog_aliases_only_in_the_service() {
        let position = position(&ProfileGift {
            id: 1,
            collection_id: 42,
            model_document_id: 99,
            relation: GiftRelation::Hosted,
        });
        assert_eq!(position.model.namespace.as_deref(), Some("tg"));
        assert_eq!(position.model.collection, "42");
        assert_eq!(position.model.model, "99");
        assert_eq!(position.model.attribute_id, "tg.model");
        assert_eq!(position.quantity, 1);
    }
}
