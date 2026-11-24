//! Bounded inventory valuation using the same gift strategy and collection preparation.
use crate::{
    GiftAttributes, GiftFloor, Strategy, ValuationContext, ValuationError, ValuationMetric,
};
use market_gifts::{Amount, CollectibleGift, CurrencyId, inventory::InventoryGift};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct InventoryValue {
    pub gift: InventoryGift,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<CollectibleGift>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Amount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct InventoryValuation {
    pub currency_id: CurrencyId,
    #[cfg_attr(feature="schema", schema(value_type=String, format=DateTime))]
    pub evaluated_at: jiff::Timestamp,
    pub stale: bool,
    pub total: Option<Amount>,
    pub priced: usize,
    pub unpriced: usize,
    pub items: Vec<InventoryValue>,
}
impl crate::ValuationHandle {
    pub async fn inventory(
        &self,
        gifts: Arc<Vec<InventoryGift>>,
        currency: CurrencyId,
        progress: tokio::sync::watch::Sender<usize>,
    ) -> Result<Arc<InventoryValuation>, ValuationError> {
        if gifts.len() > 10_000 {
            return Err(ValuationError::invalid("inventory exceeds 10000 gifts"));
        }
        let books = self.books.clone();
        self.source
            .prepare(move |snapshot| -> Result<_, ValuationError> {
                snapshot.require_currency(&currency)?;
                let context = ValuationContext::new(&snapshot, &currency, &books);
                let mut items = Vec::with_capacity(gifts.len());
                let mut total = rust_decimal::Decimal::ZERO;
                let mut priced = 0;
                // Resolve each appearance once. Never enumerate unused model/backdrop combinations.
                let resolved = snapshot.inventory_gifts(&gifts)?;
                let mut estimates = BTreeMap::new();
                for (index, (gift, resolution)) in gifts.iter().zip(resolved).enumerate() {
                    snapshot.check_cancelled()?;
                    let facts = &gift.gift;
                    let (resolved, estimate, error) = match resolution {
                        Ok(mut resolved) => {
                            resolved.id = facts.id.clone();
                            resolved.number = Some(facts.number);
                            let key = (
                                resolved.collection_id.clone(),
                                resolved.traits.model.clone(),
                                resolved.traits.backdrop.clone(),
                            );
                            let estimate = estimates
                                .entry(key)
                                .or_insert_with(|| {
                                    GiftFloor
                                        .evaluate(
                                            &GiftAttributes {
                                                collection_id: resolved.collection_id.clone(),
                                                traits: resolved.traits.clone(),
                                                number: Some(facts.number),
                                            },
                                            &context,
                                        )
                                        .map_err(|e| e.to_string())
                                        .and_then(|v| match v.final_mark {
                                            ValuationMetric::Value { value } => Ok(value),
                                            ValuationMetric::Unavailable { error } => {
                                                Err(format!("{:?}", error.code))
                                            }
                                        })
                                })
                                .clone();
                            match estimate {
                                Ok(value) => (Some(resolved), Some(value), None),
                                Err(e) => (Some(resolved), None, Some(e)),
                            }
                        }
                        Err(e) => (None, None, Some(e)),
                    };
                    if let Some(value) = &estimate {
                        total = total.checked_add(value.decimal()).ok_or_else(|| {
                            ValuationError::unavailable("portfolio total overflow")
                        })?;
                        priced += 1;
                    }
                    items.push(InventoryValue {
                        gift: gift.clone(),
                        resolved,
                        estimate,
                        error,
                    });
                    if (index + 1).is_multiple_of(25) || index + 1 == gifts.len() {
                        progress.send_replace(index + 1);
                    }
                }
                Ok(Arc::new(InventoryValuation {
                    currency_id: currency,
                    evaluated_at: jiff::Timestamp::now(),
                    stale: snapshot.sync()?.state != market_gifts::SyncState::Live,
                    total: if priced > 0 || gifts.is_empty() {
                        Some(
                            Amount::new(total.to_string())
                                .map_err(|e| ValuationError::invalid(e.to_string()))?,
                        )
                    } else {
                        None
                    },
                    priced,
                    unpriced: gifts.len() - priced,
                    items,
                }))
            })
            .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integration_tests::entity;
    use crate::test_support::MarketDataService;
    use market_gifts::{
        TelegramDocumentId, TelegramGiftDetails, TelegramGiftId, inventory::GiftRelation,
    };
    use market_hub_protocol::wire as w;

    fn gift(number: u32) -> InventoryGift {
        InventoryGift {
            gift: TelegramGiftDetails {
                id: format!("unlisted-{number}").try_into().unwrap(),
                slug: format!("Gift-{number}").try_into().unwrap(),
                number: number.try_into().unwrap(),
                collection: TelegramGiftId::new(1).unwrap(),
                model: TelegramDocumentId::new(2).unwrap(),
                backdrop: Some(3),
                pattern: Some(TelegramDocumentId::new(4).unwrap()),
            },
            relation: GiftRelation::Hosted,
        }
    }
    #[tokio::test]
    async fn inventory_uses_the_gift_strategy_and_keeps_unknown_gifts_unpriced() {
        let (_dir, mut writer) = crate::gift_floor::tests::book();
        let mut changes = Vec::new();
        // Preserve collection rules while adding Telegram identities to the fixture.
        let rows = market_replica::catalog_read::entities_cancellable(
            &writer.reader(),
            w::EntityKind::ItemType,
            None,
            &tokio_util::sync::CancellationToken::new(),
        )
        .unwrap();
        for row in rows {
            if let w::entity::Entity::ItemType(mut item) = row {
                item.aliases.push(w::ExternalId {
                    namespace: "tg".into(),
                    id: "1".into(),
                });
                changes.push(entity(w::entity::Entity::ItemType(item)));
            }
        }
        for (kind, id, alias) in [
            ("tg.model", "m", "2"),
            ("tg.backdrop", "b", "3"),
            ("tg.pattern", "symbol", "4"),
        ] {
            changes.push(entity(w::entity::Entity::AttributeOption(
                w::AttributeOption {
                    id: id.into(),
                    attribute_id: kind.into(),
                    name: id.into(),
                    aliases: vec![w::ExternalId {
                        namespace: "tg".into(),
                        id: alias.into(),
                    }],
                    ..Default::default()
                },
            )));
        }
        writer
            .apply(w::Batch {
                cursor: "aliases".into(),
                changes,
            })
            .unwrap();
        let data = MarketDataService::start(writer.reader(), 2, 1)
            .await
            .unwrap();
        let (handle, task) = crate::ValuationHandle::start(&data.arbiter(), data.reader.executor())
            .await
            .unwrap();
        let mut unknown = gift(3);
        unknown.gift.model = TelegramDocumentId::new(999).unwrap();
        let (tx, progress) = tokio::sync::watch::channel(0);
        let report = handle
            .inventory(
                Arc::new(vec![gift(1), gift(2), unknown.clone()]),
                "GRAM".into(),
                tx,
            )
            .await
            .unwrap();
        assert_eq!(report.total.as_ref().unwrap().as_str(), "36");
        assert_eq!((report.priced, report.unpriced), (2, 1));
        assert_eq!(*progress.borrow(), 3);
        assert_eq!(report.items[0].estimate.as_ref().unwrap().as_str(), "18");
        assert_eq!(
            report.items[0]
                .resolved
                .as_ref()
                .unwrap()
                .traits
                .pattern
                .as_ref()
                .unwrap()
                .as_str(),
            "symbol"
        );
        assert_eq!(report.items[0].gift.relation, GiftRelation::Hosted);
        assert!(report.items[2].error.is_some());
        let (tx, _) = tokio::sync::watch::channel(0);
        let unknown = handle
            .inventory(Arc::new(vec![unknown]), "GRAM".into(), tx)
            .await
            .unwrap();
        assert!(
            unknown.total.is_none(),
            "unpriced inventory must not be reported as zero"
        );
        let (tx, _) = tokio::sync::watch::channel(0);
        let empty = handle
            .inventory(Arc::new(vec![]), "GRAM".into(), tx)
            .await
            .unwrap();
        assert_eq!(empty.total.as_ref().unwrap().as_str(), "0");
        task.shutdown().await.unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
}
