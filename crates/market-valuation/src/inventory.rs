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
        let queued = std::time::Instant::now();
        self.source
            .prepare(move |snapshot| -> Result<_, ValuationError> {
                snapshot.require_currency(&currency)?;
                let started = std::time::Instant::now();
                tracing::info!(
                    gifts = gifts.len(),
                    admission_ms = queued.elapsed().as_millis(),
                    "portfolio valuation started"
                );
                // Resolve aliases once, then process each collection contiguously.
                // A collection's context/estimates are released before preparing the next.
                let resolved = snapshot.inventory_gifts(&gifts)?;
                let resolution_ms = started.elapsed().as_millis();
                let mut items: Vec<_> = gifts
                    .iter()
                    .map(|gift| InventoryValue {
                        gift: gift.clone(),
                        resolved: None,
                        estimate: None,
                        error: None,
                    })
                    .collect();
                let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
                let mut completed = 0;
                for (index, resolution) in resolved.into_iter().enumerate() {
                    snapshot.check_cancelled()?;
                    match resolution {
                        Ok(gift) => groups
                            .entry(gift.collection_id.clone())
                            .or_default()
                            .push((index, gift)),
                        Err(error) => {
                            items[index].error = Some(error);
                            completed += 1;
                        }
                    }
                }
                progress.send_replace(completed);
                tracing::info!(
                    gifts = gifts.len(),
                    collections = groups.len(),
                    resolution_ms,
                    "portfolio attributes resolved"
                );
                let collections = groups.len();
                let mut appearances = 0;
                let mut total = rust_decimal::Decimal::ZERO;
                let mut priced = 0;
                for (collection, entries) in groups {
                    let context = ValuationContext::new(&snapshot, &currency, &books);
                    // A failing book preparation is shared too; never retry it for every pair.
                    let prepared = context
                        .floor_statistics(&collection)
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    let mut estimates = BTreeMap::new();
                    for (index, resolved) in entries {
                        snapshot.check_cancelled()?;
                        let key = (
                            resolved.traits.model.clone(),
                            resolved.traits.backdrop.clone(),
                        );
                        let estimate = estimates
                            .entry(key)
                            .or_insert_with(|| {
                                appearances += 1;
                                prepared.clone()?;
                                GiftFloor
                                    .evaluate(
                                        &GiftAttributes {
                                            collection_id: collection.clone(),
                                            traits: resolved.traits.clone(),
                                            number: resolved.number,
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
                        let item = &mut items[index];
                        item.resolved = Some(resolved);
                        match estimate {
                            Ok(value) => {
                                total = total.checked_add(value.decimal()).ok_or_else(|| {
                                    ValuationError::unavailable("portfolio total overflow")
                                })?;
                                priced += 1;
                                item.estimate = Some(value);
                            }
                            Err(error) => item.error = Some(error),
                        }
                        completed += 1;
                        if completed.is_multiple_of(25) || completed == gifts.len() {
                            progress.send_replace(completed);
                        }
                    }
                    progress.send_replace(completed);
                }
                tracing::info!(
                    gifts = gifts.len(),
                    collections,
                    appearances,
                    priced,
                    resolution_ms,
                    elapsed_ms = started.elapsed().as_millis(),
                    "portfolio valuation completed"
                );
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

#[cfg(test)]
mod scaling_tests {
    use super::*;
    use market_gifts::{
        TelegramDocumentId, TelegramGiftDetails, TelegramGiftId, inventory::GiftRelation,
    };
    use market_hub_protocol::wire as w;
    #[tokio::test]
    async fn mixed_6000_gift_inventory_prepares_each_collection_once_and_preserves_input_order() {
        use crate::integration_tests::entity;
        use std::sync::atomic::Ordering;
        const COLLECTIONS: usize = 12; // More than the shared eight-entry trait cache.
        const MODELS: usize = 50;
        let (_dir, mut writer) = crate::gift_floor::tests::book();
        let mut changes = vec![entity(w::entity::Entity::AttributeOption(
            w::AttributeOption {
                id: "b".into(),
                attribute_id: "tg.backdrop".into(),
                name: "Backdrop".into(),
                aliases: vec![w::ExternalId {
                    namespace: "tg".into(),
                    id: "3".into(),
                }],
                ..Default::default()
            },
        ))];
        for model in 0..MODELS {
            changes.push(entity(w::entity::Entity::AttributeOption(
                w::AttributeOption {
                    id: format!("model-{model}"),
                    attribute_id: "tg.model".into(),
                    name: "Model".into(),
                    aliases: vec![w::ExternalId {
                        namespace: "tg".into(),
                        id: (1000 + model).to_string(),
                    }],
                    ..Default::default()
                },
            )));
        }
        for collection in 0..COLLECTIONS {
            let id = format!("batch-{collection}");
            changes.push(entity(w::entity::Entity::ItemType(w::ItemType {
                id: id.clone(),
                name: "Batch".into(),
                quantity_step: "1".into(),
                aliases: vec![w::ExternalId {
                    namespace: "tg".into(),
                    id: (100 + collection).to_string(),
                }],
                attributes: [
                    (
                        "tg.model".into(),
                        w::AttributeRule {
                            allowed_values: (collection % 2 == 0).then(|| {
                                w::AllowedAttributeValues {
                                    ids: (0..MODELS).map(|m| format!("model-{m}")).collect(),
                                }
                            }),
                            required: false,
                        },
                    ),
                    (
                        "tg.backdrop".into(),
                        w::AttributeRule {
                            allowed_values: Some(w::AllowedAttributeValues {
                                ids: vec!["b".into()],
                            }),
                            required: false,
                        },
                    ),
                ]
                .into(),
                ..Default::default()
            })));
            for model in 0..MODELS {
                let mut change = crate::gift_floor::tests::ask(
                    &format!("batch-{collection}-{model}"),
                    &format!("model-{model}"),
                    "b",
                    "10",
                    1,
                    None,
                );
                if let Some(w::change::Change::Upsert(w::Entity {
                    entity: Some(w::entity::Entity::Order(order)),
                })) = &mut change.change
                {
                    order.book.as_mut().unwrap().item_type_id = id.clone();
                }
                changes.push(change);
            }
        }
        writer
            .apply(w::Batch {
                cursor: "large-inventory".into(),
                changes,
            })
            .unwrap();
        let data = crate::test_support::MarketDataService::start(writer.reader(), 2, 1)
            .await
            .unwrap();
        let (handle, task) = crate::ValuationHandle::start(&data.arbiter(), data.reader.executor())
            .await
            .unwrap();
        let gifts: Arc<Vec<_>> = Arc::new(
            (0..6000)
                .map(|i| InventoryGift {
                    gift: TelegramGiftDetails {
                        id: format!("owned-{i}").try_into().unwrap(),
                        slug: format!("Gift-{}", i + 1).try_into().unwrap(),
                        number: (i as u32 + 1).try_into().unwrap(),
                        collection: TelegramGiftId::new((100 + i % COLLECTIONS) as i64).unwrap(),
                        model: TelegramDocumentId::new((1000 + (i / COLLECTIONS) % MODELS) as i64)
                            .unwrap(),
                        backdrop: Some(3),
                        pattern: None,
                    },
                    relation: GiftRelation::Owned,
                })
                .collect(),
        );
        let (tx, progress) = tokio::sync::watch::channel(0);
        let started = std::time::Instant::now();
        let report = handle
            .inventory(gifts.clone(), "GRAM".into(), tx)
            .await
            .unwrap();
        eprintln!(
            "6000 gifts / 12 collections / 600 appearances: {:?}, {} statistics builds",
            started.elapsed(),
            handle.books.preparations.load(Ordering::Relaxed)
        );
        assert_eq!(report.total.as_ref().unwrap().as_str(), "60000");
        assert_eq!((report.priced, report.unpriced), (6000, 0));
        assert_eq!(*progress.borrow(), 6000);
        assert_eq!(
            handle.books.preparations.load(Ordering::Relaxed),
            COLLECTIONS
        );
        for (input, output) in gifts.iter().zip(&report.items) {
            assert_eq!(input.gift.id, output.gift.gift.id);
            assert_eq!(output.estimate.as_ref().unwrap().as_str(), "10");
        }
        // A pinned context also survives eviction by unrelated cache consumers.
        handle
            .source
            .prepare(move |snapshot| -> Result<(), ValuationError> {
                let books = crate::floors::BookCache::default();
                let currency = "GRAM".into();
                let context = ValuationContext::new(&snapshot, &currency, &books);
                let collection = "batch-0".into();
                let first = context.floor_statistics(&collection)?;
                for i in 1..COLLECTIONS {
                    books.get(
                        &snapshot,
                        &crate::source::BookScope {
                            collection: format!("batch-{i}").try_into().unwrap(),
                            currency: currency.clone(),
                        },
                    )?;
                }
                assert!(Arc::ptr_eq(&first, &context.floor_statistics(&collection)?));
                assert!(Arc::ptr_eq(
                    &first,
                    &context.collection_statistics(&collection)?
                ));
                assert_eq!(books.preparations.load(Ordering::Relaxed), COLLECTIONS);
                Ok(())
            })
            .await
            .unwrap()
            .unwrap();
        task.shutdown().await.unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
}
