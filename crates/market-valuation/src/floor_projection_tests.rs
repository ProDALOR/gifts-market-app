use super::*;
use crate::{floors::BookCache, market::ValuationRead};
use crate::{
    gift_floor::tests::{ask, book},
    integration_tests::entity,
    test_support::MarketDataService,
};
use market_hub_protocol::wire as w;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

fn request() -> ValuationQuery {
    ValuationQuery {
        currency_id: "GRAM".into(),
        strategies: vec![StrategyId::new(CatalogFloor::ID).unwrap()],
        period: None,
        target: ValuationTarget::Catalog(CatalogSelection {
            collection_ids: vec!["gift".into()],
            details: Some(CatalogDetails {
                collection_id: "gift".into(),
                pair: Some(PairSelection {
                    model_id: "m".into(),
                    backdrop_id: "b".into(),
                }),
            }),
        }),
    }
}
fn catalog(report: &ValuationReport) -> &CatalogFloorValuation {
    match &report.results[0] {
        StrategyOutcome::Value(StrategyValuation::CatalogFloor(data)) => data,
        result => panic!("unexpected result: {result:?}"),
    }
}
fn scope(request: &mut ValuationQuery) -> &mut CatalogSelection {
    let ValuationTarget::Catalog(scope) = &mut request.target else {
        panic!()
    };
    scope
}

#[tokio::test]
async fn batch_reuses_gift_statistics_and_status_changes_do_not_rebuild_price_data() {
    let (_dir, mut writer) = book();
    let reads =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = source::ValuationSource::new(reads.handle());
    let cache = Arc::new(BookCache::default());
    let prepare = |previous: Option<Arc<ValuationRead>>| {
        let cache = cache.clone();
        source.prepare(move |snapshot| {
            ValuationRead::read(&snapshot, &request().prepare()?, &cache, previous.as_ref())
        })
    };
    let first = prepare(None).await.unwrap().unwrap();
    assert_eq!(
        catalog(&first.report).collections[0]
            .floor
            .price
            .value()
            .unwrap()
            .as_str(),
        "3"
    );
    let traits = catalog(&first.report).collections[0]
        .traits
        .as_ref()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&traits.patterns[&PatternId::from("symbol")].price).unwrap()["error"]
            ["code"],
        "not_enough_data"
    );
    assert_eq!(
        traits.patterns[&PatternId::from("symbol")].evidence.gifts,
        0
    );
    assert_eq!(
        traits.models[&ModelId::from("m")]
            .price
            .value()
            .unwrap()
            .as_str(),
        "10"
    );
    assert_eq!(
        traits.backdrops[&BackdropId::from("b")]
            .price
            .value()
            .unwrap()
            .as_str(),
        "11"
    );
    assert_eq!(
        traits
            .pair
            .as_ref()
            .unwrap()
            .floor
            .price
            .value()
            .unwrap()
            .as_str(),
        "100"
    );
    writer
        .apply(w::Batch {
            cursor: "unrelated".into(),
            changes: vec![entity(w::entity::Entity::Currency(w::Currency {
                id: "OTHER".into(),
                name: "Other".into(),
                decimals: 9,
            }))],
        })
        .unwrap();
    let same = prepare(Some(first.clone())).await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &same));
    writer.mark(market_replica::SyncState::Stale);
    let stale = prepare(Some(first.clone())).await.unwrap().unwrap();
    assert!(stale.report.stale);
    let (
        StrategyOutcome::Value(StrategyValuation::CatalogFloor(before)),
        StrategyOutcome::Value(StrategyValuation::CatalogFloor(after)),
    ) = (&first.report.results[0], &stale.report.results[0])
    else {
        panic!()
    };
    assert!(
        Arc::ptr_eq(before, after),
        "availability changes share the complete catalog result"
    );
    assert!(Arc::ptr_eq(&first.statistics()[0], &stale.statistics()[0]));
    writer.mark(market_replica::SyncState::Live);
    let live = prepare(Some(stale)).await.unwrap().unwrap();
    assert!(!live.report.stale);
    assert!(Arc::ptr_eq(&first.statistics()[0], &live.statistics()[0]));
    let missing = source
        .prepare(move |snapshot| {
            let mut query = request();
            scope(&mut query)
                .details
                .as_mut()
                .unwrap()
                .pair
                .as_mut()
                .unwrap()
                .model_id = "removed".into();
            ValuationRead::read(&snapshot, &query.prepare()?, &cache, None)
        })
        .await
        .unwrap()
        .unwrap();
    assert!(
        catalog(&missing.report).collections[0]
            .floor
            .price
            .value()
            .is_some()
    );
    assert_eq!(
        serde_json::to_value(
            &catalog(&missing.report).collections[0]
                .traits
                .as_ref()
                .unwrap()
                .pair
                .as_ref()
                .unwrap()
                .floor
                .price
        )
        .unwrap()["error"]["code"],
        "missing_trait"
    );
    reads.shutdown().await;
}

#[tokio::test]
async fn watches_expire_without_updates_survive_cancelled_next_and_release_capacity() {
    let (_dir, mut writer) = book();
    writer
        .apply(w::Batch {
            cursor: "expiring".into(),
            changes: vec![ask(
                "cheap",
                "m",
                "b",
                "1",
                456,
                Some(jiff::Timestamp::now().as_millisecond() + 1500),
            )],
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let mut watch = handle.watch(request()).await.unwrap();
    assert_eq!(
        catalog(&watch.current()).collections[0]
            .floor
            .price
            .value()
            .unwrap()
            .as_str(),
        "1"
    );
    let next = tokio::time::timeout(Duration::from_secs(4), watch.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        catalog(&next).collections[0]
            .floor
            .price
            .value()
            .unwrap()
            .as_str(),
        "3"
    );
    writer
        .apply(w::Batch {
            cursor: "new-floor".into(),
            changes: vec![ask("new", "m", "b", "2", 789, None)],
        })
        .unwrap();
    // Cancellation during debounce must retain the fact that the source changed.
    assert!(
        tokio::time::timeout(Duration::from_millis(30), watch.changed())
            .await
            .is_err()
    );
    assert_eq!(
        catalog(
            &tokio::time::timeout(Duration::from_secs(2), watch.changed())
                .await
                .unwrap()
                .unwrap()
        )
        .collections[0]
            .floor
            .price
            .value()
            .unwrap()
            .as_str(),
        "2"
    );
    let mut watches = vec![];
    for _ in 0..31 {
        watches.push(handle.watch(request()).await.unwrap());
    }
    assert_eq!(
        handle.watch(request()).await.err().unwrap().code(),
        ValuationErrorCode::CapacityExceeded
    );
    watches.pop();
    watches.push(handle.watch(request()).await.unwrap());
    task.shutdown().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), watch.changed())
            .await
            .unwrap()
            .is_err()
    );
    drop(watches);
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[test]
fn batch_scope_is_explicit_and_bounded() {
    let mut query = request();
    assert!(query.validate().is_ok());
    scope(&mut query).collection_ids.push("gift".into());
    assert!(query.validate().is_err());
    scope(&mut query).collection_ids = vec!["other".into()];
    assert!(query.validate().is_err());
    scope(&mut query).details = None;
    assert!(query.validate().is_ok());
    scope(&mut query).collection_ids = (0..9)
        .map(|n| CollectionId::new(format!("gift{n}")).unwrap())
        .collect();
    assert!(query.validate().is_err());
}

#[tokio::test]
async fn a_batch_refresh_keeps_other_collections_calculation_times() {
    let (_dir, mut writer) = book();
    writer
        .apply(w::Batch {
            cursor: "other-catalog".into(),
            changes: vec![entity(w::entity::Entity::ItemType(w::ItemType {
                id: "other".into(),
                name: "Other".into(),
                quantity_step: "1".into(),
                aliases: vec![w::ExternalId {
                    namespace: "tg.slug".into(),
                    id: "Other".into(),
                }],
                ..Default::default()
            }))],
        })
        .unwrap();
    let reads =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = source::ValuationSource::new(reads.handle());
    let cache = Arc::new(BookCache::default());
    let read = |previous: Option<Arc<ValuationRead>>| {
        let cache = cache.clone();
        source.prepare(move |snapshot| {
            let mut query = request();
            scope(&mut query).collection_ids.push("other".into());
            ValuationRead::read(&snapshot, &query.prepare()?, &cache, previous.as_ref())
        })
    };
    let first = read(None).await.unwrap().unwrap();
    let mut change = ask("other", "m", "b", "2", 700, None);
    if let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(order)),
    })) = &mut change.change
    {
        order.book.as_mut().unwrap().item_type_id = "other".into();
    }
    writer
        .apply(w::Batch {
            cursor: "other-order".into(),
            changes: vec![change],
        })
        .unwrap();
    let next = read(Some(first.clone())).await.unwrap().unwrap();
    assert!(!Arc::ptr_eq(&first, &next));
    assert_eq!(
        catalog(&first.report).collections[0].evaluated_at,
        catalog(&next.report).collections[0].evaluated_at
    );
    assert!(
        catalog(&next.report).collections[1].evaluated_at
            > catalog(&first.report).collections[1].evaluated_at
    );
    assert!(Arc::ptr_eq(&first.statistics()[0], &next.statistics()[0]));
    reads.shutdown().await;
}

#[tokio::test]
async fn symbol_floors_deduplicate_gifts_and_follow_catalog_and_order_changes() {
    let (_dir, mut writer) = book();
    fn symbolic(id: &str, price: &str) -> w::Change {
        let mut change = ask(id, "m", "b", price, 700, None);
        let Some(w::change::Change::Upsert(w::Entity {
            entity: Some(w::entity::Entity::Order(order)),
        })) = &mut change.change
        else {
            panic!()
        };
        order.item_id = Some("same-symbol-gift".into());
        order.attributes.insert(
            "tg.pattern".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::OptionId("symbol".into())),
                })),
            },
        );
        change
    }
    writer
        .apply(w::Batch {
            cursor: "symbols".into(),
            changes: vec![symbolic("symbol-one", "6"), symbolic("symbol-two", "5")],
        })
        .unwrap();
    let reads =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = source::ValuationSource::new(reads.handle());
    let cache = Arc::new(BookCache::default());
    let read = || {
        let cache = cache.clone();
        source.prepare(move |snapshot| {
            ValuationRead::read(&snapshot, &request().prepare()?, &cache, None)
        })
    };
    let first = read().await.unwrap().unwrap();
    let symbol = &catalog(&first.report).collections[0]
        .traits
        .as_ref()
        .unwrap()
        .patterns[&PatternId::from("symbol")];
    assert_eq!(symbol.price.value().unwrap().as_str(), "5");
    assert_eq!(symbol.evidence.gifts, 1);
    assert_eq!(symbol.evidence.listings, 2);
    writer
        .apply(w::Batch {
            cursor: "symbol-repriced".into(),
            changes: vec![symbolic("symbol-one", "4")],
        })
        .unwrap();
    let next = read().await.unwrap().unwrap();
    assert_eq!(
        catalog(&next.report).collections[0]
            .traits
            .as_ref()
            .unwrap()
            .patterns[&PatternId::from("symbol")]
            .price
            .value()
            .unwrap()
            .as_str(),
        "4"
    );
    assert_eq!(
        symbol.price.value().unwrap().as_str(),
        "5",
        "the prior snapshot remains unchanged"
    );
    writer
        .apply(w::Batch {
            cursor: "symbol-removed".into(),
            changes: vec![w::Change {
                change: Some(w::change::Change::Remove(w::Removal {
                    key: Some(w::Key {
                        kind: w::EntityKind::AttributeOption as i32,
                        id: "symbol".into(),
                        venue_id: String::new(),
                    }),
                    observed_at_ms: None,
                })),
            }],
        })
        .unwrap();
    let removed = read().await.unwrap().unwrap();
    assert!(
        catalog(&removed.report).collections[0]
            .traits
            .as_ref()
            .unwrap()
            .patterns
            .is_empty()
    );
    reads.shutdown().await;
}
