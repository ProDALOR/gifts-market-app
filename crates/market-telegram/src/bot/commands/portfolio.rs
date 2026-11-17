use super::{QuoteCurrency, Reply, render};
use crate::bot::{BotError, GiftSource, InventoryError, ProfileGift};
use market_actors::MarketDataHandle;
use market_gifts::{
    InvalidValue, TelegramDocumentId, TelegramGiftId,
    valuation::{ModelLookup, ModelPosition, ValuationRequest},
};
fn position(gift: &ProfileGift) -> Result<ModelPosition, InvalidValue> {
    Ok(ModelPosition {
        model: ModelLookup::Telegram {
            gift_id: TelegramGiftId::new(gift.collection_id)?,
            document_id: TelegramDocumentId::new(gift.model_document_id)?,
        },
        quantity: 1,
    })
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
    let positions = inventory
        .gifts
        .iter()
        .map(position)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| BotError::InvalidResponse)?;
    Ok(
        match market
            .value_models(ValuationRequest {
                positions,
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
        })
        .unwrap();
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
