use super::{Reply, render};
use market_actors::MarketDataHandle;
use market_gifts::{
    CurrencyId,
    valuation::{ModelLookup, ModelPosition, ValuationRequest},
};
use market_telegram::{BotError, GiftSource, InventoryError, ProfileGift};
fn position(gift: &ProfileGift) -> ModelPosition {
    ModelPosition {
        model: ModelLookup::Telegram {
            gift_id: gift.collection_id,
            document_id: gift.model_document_id,
        },
        quantity: 1,
    }
}
pub(super) async fn execute<S: GiftSource>(
    market: &MarketDataHandle,
    source: &S,
    currency: CurrencyId,
) -> Result<Reply, BotError> {
    let inventory = match source.inventory().await {
        Ok(inventory) => inventory,
        Err(InventoryError::Authorization) => return Err(BotError::Authorization),
        Err(error) => return Ok(render::inventory_error(error)),
    };
    let positions = inventory.gifts.iter().map(position).collect();
    Ok(
        match market
            .value_models(ValuationRequest {
                positions,
                currency_id: currency,
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
    use market_gifts::{TelegramDocumentId, TelegramGiftId};
    use market_telegram::GiftRelation;
    #[test]
    fn telegram_identifiers_become_explicit_catalog_aliases_only_in_the_service() {
        let position = position(&ProfileGift {
            id: 1,
            collection_id: TelegramGiftId::new(42).unwrap(),
            model_document_id: TelegramDocumentId::new(99).unwrap(),
            relation: GiftRelation::Hosted,
        });
        assert_eq!(
            position.model,
            ModelLookup::Telegram {
                gift_id: TelegramGiftId::new(42).unwrap(),
                document_id: TelegramDocumentId::new(99).unwrap()
            }
        );
        assert_eq!(position.quantity, 1);
    }
}
