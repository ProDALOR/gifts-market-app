use super::*;
use crate::floors::BookCache;
use crate::integration_tests::{entity, fixture, hold};
use crate::source::BookScope;
use crate::test_support::MarketDataService;
use crate::*;
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) fn ask(
    id: &str,
    model: &str,
    backdrop: &str,
    price: &str,
    number: i64,
    expiry: Option<i64>,
) -> w::Change {
    entity(w::entity::Entity::Order(w::Order {
        id: id.into(),
        book: Some(w::BookKey {
            venue_id: "venue".into(),
            item_type_id: "gift".into(),
            currency_id: "GRAM".into(),
        }),
        mode: 1,
        price: price.into(),
        quantity: "1".into(),
        item_id: Some(format!("gift-{id}")),
        expires_at_ms: expiry,
        attributes: [
            (
                "tg.model".into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId(model.into())),
                    })),
                },
            ),
            (
                "tg.backdrop".into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId(backdrop.into())),
                    })),
                },
            ),
            (
                "tg.number".into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::Integer(number)),
                    })),
                },
            ),
        ]
        .into(),
        ..Default::default()
    }))
}
pub(crate) fn book() -> (tempfile::TempDir, market_replica::ReplicaWriter) {
    let (dir, mut writer) = fixture();
    let mut changes = vec![w::Change {
        change: Some(w::change::Change::Remove(w::Removal {
            key: Some(w::Key {
                kind: w::EntityKind::Order as i32,
                id: "order".into(),
                venue_id: "venue".into(),
            }),
            observed_at_ms: None,
        })),
    }];
    for (attribute, ids) in [
        ("tg.model", vec!["m", "otherm"]),
        ("tg.backdrop", vec!["b", "otherb"]),
        ("tg.pattern", vec!["symbol"]),
    ] {
        changes.push(entity(w::entity::Entity::Attribute(w::Attribute {
            id: attribute.into(),
            name: attribute.into(),
            value_type: w::AttributeType::Enum as i32,
            ..Default::default()
        })));
        for id in ids {
            changes.push(entity(w::entity::Entity::AttributeOption(
                w::AttributeOption {
                    id: id.into(),
                    attribute_id: attribute.into(),
                    name: id.into(),
                    ..Default::default()
                },
            )));
        }
    }
    changes.push(entity(w::entity::Entity::Attribute(w::Attribute {
        id: "tg.number".into(),
        name: "Number".into(),
        value_type: w::AttributeType::Integer as i32,
        ..Default::default()
    })));
    changes.push(entity(w::entity::Entity::ItemType(w::ItemType {
        id: "gift".into(),
        name: "Gift".into(),
        quantity_step: "1".into(),
        aliases: vec![w::ExternalId {
            namespace: "tg.slug".into(),
            id: "Gift".into(),
        }],
        attributes: [
            (
                "tg.model".into(),
                w::AttributeRule {
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec!["m".into(), "otherm".into()],
                    }),
                    required: false,
                },
            ),
            (
                "tg.backdrop".into(),
                w::AttributeRule {
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec!["b".into(), "otherb".into()],
                    }),
                    required: false,
                },
            ),
            ("tg.number".into(), w::AttributeRule::default()),
            ("tg.pattern".into(), w::AttributeRule::default()),
        ]
        .into(),
    })));
    for (group, model, backdrop, price) in [
        ("base", "otherm", "otherb", "3"),
        ("model", "m", "otherb", "10"),
        ("backdrop", "otherm", "b", "11"),
        ("pair", "m", "b", "100"),
    ] {
        for i in 0..3 {
            changes.push(ask(
                &format!("{group}{i}"),
                model,
                backdrop,
                price,
                if group == "pair" && i == 0 {
                    123
                } else {
                    1000 + changes.len() as i64
                },
                None,
            ));
        }
    }
    writer
        .apply(w::Batch {
            cursor: "initial".into(),
            changes,
        })
        .unwrap();
    (dir, writer)
}
fn request() -> ValuationQuery {
    ValuationQuery {
        target: ValuationTarget::Gift {
            gift: GiftLookup::Catalog {
                collection_id: "gift".into(),
                number: GiftNumber::new(123).unwrap(),
            },
        },
        currency_id: "GRAM".into(),
        strategies: vec![StrategyId::new("gift_floor").unwrap()],
        period: None,
    }
}
struct Evaluation {
    data: GiftFloorValuation,
    stale: bool,
}
impl std::ops::Deref for Evaluation {
    type Target = GiftFloorValuation;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
async fn evaluate(handle: &ValuationHandle) -> Result<Evaluation, ValuationError> {
    let report = handle.query(request()).await?;
    let StrategyOutcome::Value(StrategyValuation::GiftFloor(data)) = &report.results[0] else {
        panic!("unexpected gift outcome: {:?}", report.results)
    };
    Ok(Evaluation {
        data: data.clone(),
        stale: report.stale,
    })
}
async fn stop(data: MarketDataService, task: ValuationTask) {
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn unified_report_uses_combination_floor_and_full_precision_then_reacts_to_book_changes() {
    let (_dir, mut writer) = book();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let first = evaluate(&handle).await.unwrap();
    assert_eq!(first.collection_floor.value().unwrap().as_str(), "3");
    assert_eq!(first.model_floor.value().unwrap().as_str(), "10");
    assert_eq!(first.backdrop_floor.value().unwrap().as_str(), "11");
    assert_eq!(first.model_backdrop_floor.value().unwrap().as_str(), "100");
    assert_eq!(first.model_multiplier.value().unwrap().as_str(), "3.33");
    assert_eq!(first.backdrop_multiplier.value().unwrap().as_str(), "3.67");
    assert_eq!(first.final_mark.value().unwrap().as_str(), "36.67");
    assert_eq!(first.evidence.collection.listings, 12);
    writer
        .apply(w::Batch {
            cursor: "replaced".into(),
            changes: vec![ask("pair0", "m", "b", "20.125", 123, None)],
        })
        .unwrap();
    let next = evaluate(&handle).await.unwrap();
    assert_eq!(
        next.model_backdrop_floor.value().unwrap().as_str(),
        "20.125"
    );
    assert_eq!(next.final_mark.value().unwrap().as_str(), "20.12");
    assert_eq!(first.final_mark.value().unwrap().as_str(), "36.67");
    stop(data, task).await;
}

#[tokio::test]
async fn statistics_cache_ignores_unrelated_metadata_and_tracks_backdrop_dependencies() {
    let (_dir, mut writer) = book();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let books = BookCache::default();
    let scope = BookScope {
        collection: "gift".into(),
        currency: "GRAM".into(),
    };
    let first = books.get(&data.reader.snapshot().unwrap(), &scope).unwrap();
    writer
        .apply(w::Batch {
            cursor: "unrelated".into(),
            changes: vec![entity(w::entity::Entity::Currency(w::Currency {
                id: "XTR".into(),
                name: "Stars".into(),
                decimals: 0,
            }))],
        })
        .unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &books.get(&data.reader.snapshot().unwrap(), &scope).unwrap()
    ));
    writer
        .apply(w::Batch {
            cursor: "metadata".into(),
            changes: vec![entity(w::entity::Entity::AttributeOption(
                w::AttributeOption {
                    id: "b".into(),
                    attribute_id: "tg.backdrop".into(),
                    name: "Renamed backdrop".into(),
                    ..Default::default()
                },
            ))],
        })
        .unwrap();
    assert!(!Arc::ptr_eq(
        &first,
        &books.get(&data.reader.snapshot().unwrap(), &scope).unwrap()
    ));
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn unchanged_statistics_survive_thirty_seconds_and_rebuild_at_the_safety_age() {
    let (_dir, writer) = book();
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    tokio::time::pause();
    let books = BookCache::default();
    let scope = BookScope {
        collection: "gift".into(),
        currency: "GRAM".into(),
    };
    let first = books.get(&data.reader.snapshot().unwrap(), &scope).unwrap();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(Arc::ptr_eq(
        &first,
        &books.get(&data.reader.snapshot().unwrap(), &scope).unwrap()
    ));
    tokio::time::advance(Duration::from_secs(270)).await;
    assert!(!Arc::ptr_eq(
        &first,
        &books.get(&data.reader.snapshot().unwrap(), &scope).unwrap()
    ));
    tokio::time::resume();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn cross_venue_listings_and_broad_criteria_cannot_satisfy_distinct_gift_requirements() {
    let (_dir, mut writer) = book();
    // All pair listings identify the same collectible, despite different venues and prices.
    let mut changes = vec![entity(w::entity::Entity::Venue(w::Venue {
        id: "second".into(),
        name: "Second venue".into(),
    }))];
    for i in 0..3 {
        let mut listing = ask(&format!("pair{i}"), "m", "b", "100", 123, None);
        let Some(w::change::Change::Upsert(row)) = &mut listing.change else {
            unreachable!()
        };
        let Some(w::entity::Entity::Order(order)) = &mut row.entity else {
            unreachable!()
        };
        order.item_id = Some("same-gift".into());
        changes.push(listing);
    }
    let mut second = ask("cross-listed", "m", "b", "95", 123, None);
    let Some(w::change::Change::Upsert(row)) = &mut second.change else {
        unreachable!()
    };
    let Some(w::entity::Entity::Order(order)) = &mut row.entity else {
        unreachable!()
    };
    order.item_id = Some("same-gift".into());
    order.book.as_mut().unwrap().venue_id = "second".into();
    changes.push(second);
    // The model-only/baseline/backdrop groups become unidentified broad asks.
    for group in ["model", "backdrop", "base"] {
        for i in 0..3 {
            let (model, backdrop) = match group {
                "model" => ("m", "otherb"),
                "backdrop" => ("otherm", "b"),
                _ => ("otherm", "otherb"),
            };
            let mut broad = ask(
                &format!("{group}{i}"),
                model,
                backdrop,
                "10",
                1000 + i,
                None,
            );
            let Some(w::change::Change::Upsert(row)) = &mut broad.change else {
                unreachable!()
            };
            let Some(w::entity::Entity::Order(order)) = &mut row.entity else {
                unreachable!()
            };
            order.item_id = None;
            order.attributes.remove("tg.number");
            changes.push(broad);
        }
    }
    writer
        .apply(w::Batch {
            changes,
            cursor: "duplicates".into(),
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let mark = evaluate(&handle).await.unwrap();
    assert_eq!(
        mark.evidence.collection,
        FloorEvidence {
            listings: 13,
            gifts: 1
        }
    );
    assert_eq!(
        mark.evidence.model,
        FloorEvidence {
            listings: 7,
            gifts: 1
        }
    );
    assert_eq!(
        mark.evidence.model_backdrop,
        FloorEvidence {
            listings: 4,
            gifts: 1
        }
    );
    assert_eq!(mark.model_backdrop_floor.value().unwrap().as_str(), "95");
    let ValuationMetric::Unavailable { error } = &mark.final_mark else {
        panic!("duplicate listings cannot produce a mark")
    };
    assert_eq!(error.code, ValuationIssueCode::NotEnoughData);
    assert_eq!(error.available_samples, Some(1));
    assert_eq!(error.required_samples, Some(3));
    assert_eq!(error.dependency, Some(ValuationDependency::ModelMultiplier));
    stop(data, task).await;
}

#[tokio::test]
async fn cached_statistics_expire_with_the_earliest_ask_without_a_replica_update() {
    let (_dir, mut writer) = book();
    let expires = jiff::Timestamp::now().as_millisecond() + 1000;
    writer
        .apply(w::Batch {
            cursor: "expiring".into(),
            changes: vec![ask("cheap", "m", "b", "1", 500, Some(expires))],
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    assert_eq!(
        evaluate(&handle)
            .await
            .unwrap()
            .final_mark
            .value()
            .unwrap()
            .as_str(),
        "1"
    );
    tokio::time::sleep(Duration::from_millis(
        (expires - jiff::Timestamp::now().as_millisecond()).max(0) as u64 + 30,
    ))
    .await;
    let refreshed = evaluate(&handle).await.unwrap();
    assert_eq!(refreshed.collection_floor.value().unwrap().as_str(), "3");
    assert_eq!(refreshed.final_mark.value().unwrap().as_str(), "36.67");
    stop(data, task).await;
}

#[tokio::test]
async fn gift_actor_shutdown_cancels_a_request_waiting_for_the_shared_reader() {
    let (_dir, writer) = book();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let held = hold(&data.reader).await;
    let pending = tokio::spawn(async move { evaluate(&handle).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    task.shutdown().await.unwrap();
    assert!(pending.await.unwrap().is_err());
    drop(held);
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn strategy_display_preserves_typed_values_precision_and_structured_unavailability() {
    let (_dir, mut writer) = book();
    writer
        .apply(w::Batch {
            cursor: "presentation".into(),
            changes: vec![ask("pair0", "m", "b", "20.125", 123, None)],
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let result = evaluate(&handle).await.unwrap();
    let mut output = result.data.clone();
    output.model_floor = ValuationMetric::Value {
        value: Amount::new("10000000000000000.000000001").unwrap(),
    };
    output.backdrop_multiplier = ValuationMetric::Unavailable {
        error: ValuationIssue {
            code: ValuationIssueCode::NotEnoughData,
            message: "More observations are needed".into(),
            required_samples: Some(3),
            available_samples: Some(1),
            dependency: None,
        },
    };
    let currency = CurrencyId::from("GRAM");
    let display =
        StrategyOutcome::Value(StrategyValuation::GiftFloor(output.clone())).display(&currency);
    let ids = display
        .fields
        .iter()
        .map(|field| field.id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), display.fields.len());
    assert_eq!(
        display.fields.iter().filter(|field| field.primary).count(),
        1
    );
    let field = |id| display.fields.iter().find(|field| field.id == id).unwrap();
    assert!(field("final_mark").primary);
    let crate::DisplayValue::Currency { amount, .. } = &field("final_mark").value else {
        panic!("money");
    };
    assert_eq!(amount, "20.12");
    let crate::DisplayValue::Currency { amount, .. } = &field("model_floor").value else {
        panic!("money");
    };
    assert_eq!(amount, "10000000000000000.000000001");
    let crate::DisplayValue::Unavailable {
        label,
        message,
        detail,
    } = &field("backdrop_multiplier").value
    else {
        panic!("error");
    };
    assert_eq!(*label, "Not enough data");
    assert_eq!(message, "More observations are needed");
    assert_eq!(detail.as_deref(), Some("1 / 3 required samples"));
    assert_eq!(
        output.model_floor.value().unwrap().as_str(),
        "10000000000000000.000000001"
    );
    stop(data, task).await;
}

#[tokio::test]
async fn mark_freshness_tracks_delayed_data_without_putting_sync_in_calculations() {
    let (_directory, writer) = book();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let live = evaluate(&handle).await.unwrap();
    assert!(!live.stale);
    writer.mark(market_replica::SyncState::Stale);
    let delayed = evaluate(&handle).await.unwrap();
    assert!(delayed.stale);
    assert_eq!(live.final_mark.value(), delayed.final_mark.value());
    let value = serde_json::to_value(&delayed.data).unwrap();
    assert!(value.get("sync").is_none());
    assert!(value.get("revision").is_none());
    writer.mark(market_replica::SyncState::Live);
    assert!(!evaluate(&handle).await.unwrap().stale);
    assert!(
        delayed.stale,
        "a previously prepared mark stays marked as delayed"
    );
    stop(data, task).await;
}
