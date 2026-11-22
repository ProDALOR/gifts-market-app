use super::*;
use crate::{
    gift_floor::tests::{ask, book},
    integration_tests::entity,
    source::ValuationSource,
};
use market_gifts::*;
use market_hub_protocol::wire as w;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn summaries_do_not_prepare_traits_and_survive_a_catalog_sweep() {
    let (_dir, mut writer) = book();
    writer
        .apply(w::Batch {
            cursor: "more-collections".into(),
            changes: (0..20)
                .map(|i| {
                    entity(w::entity::Entity::ItemType(w::ItemType {
                        id: format!("other-{i}"),
                        name: format!("Other {i}"),
                        quantity_step: "1".into(),
                        ..Default::default()
                    }))
                })
                .collect(),
        })
        .unwrap();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    source
        .prepare(|snapshot| {
            let cache = BookCache::default();
            let scope = BookScope {
                collection: "gift".into(),
                currency: "GRAM".into(),
            };
            let first = cache.collection(&snapshot, &scope).unwrap().statistics;
            assert!(!first.traits_prepared());
            assert!(
                first.known_models.is_empty()
                    && first.known_backdrops.is_empty()
                    && first.known_patterns.is_empty()
            );
            assert!(
                first.models.is_empty() && first.backdrops.is_empty() && first.patterns.is_empty()
            );
            assert_eq!(first.collection.count, 12);
            for i in 0..20 {
                cache
                    .collection(
                        &snapshot,
                        &BookScope {
                            collection: CollectionId::new(format!("other-{i}")).unwrap(),
                            currency: "GRAM".into(),
                        },
                    )
                    .unwrap();
            }
            assert!(Arc::ptr_eq(
                &first,
                &cache.collection(&snapshot, &scope).unwrap().statistics
            ));
            let books = cache.lock().unwrap();
            assert_eq!(books.books.len(), 21);
            assert!(
                books.books.values().all(|entry| entry.traits.is_none()),
                "Catalog summaries never populate the selected-trait cache"
            );
        })
        .await
        .unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn requested_pairs_share_cached_trait_observation_time_until_the_minimum_refresh() {
    let (_dir, mut writer) = book();
    let expiry = jiff::Timestamp::now().as_millisecond() + 1000;
    writer
        .apply(w::Batch {
            cursor: "expiring-pair".into(),
            changes: vec![ask("pair-expiry", "m", "b", "1", 999, Some(expiry))],
        })
        .unwrap();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let cache = Arc::new(BookCache::default());
    let prepare = |cache: Arc<BookCache>| {
        source.prepare(move |snapshot| {
            let currency = "GRAM".into();
            let context = crate::ValuationContext::new(&snapshot, &currency, &cache);
            let stats = context.floor_statistics(&"gift".into()).unwrap();
            assert_eq!(stats.models.len(), 2);
            assert_eq!(stats.backdrops.len(), 2);
            let pair = stats
                .pair_floor(&snapshot, &"m".into(), &"b".into())
                .unwrap();
            assert!(Arc::ptr_eq(
                &stats,
                &context.floor_statistics(&"gift".into()).unwrap()
            ));
            assert_eq!(
                cache
                    .lock()
                    .unwrap()
                    .books
                    .values()
                    .filter(|entry| entry.traits.is_some())
                    .count(),
                1
            );
            pair
        })
    };
    let first = prepare(cache.clone()).await.unwrap();
    assert_eq!(first.price.value().unwrap().as_str(), "1");
    let remaining = (expiry - jiff::Timestamp::now().as_millisecond()).max(0) as u64;
    tokio::time::sleep(std::time::Duration::from_millis(remaining + 30)).await;
    let retained = prepare(cache.clone()).await.unwrap();
    assert_eq!(
        retained.price.value().unwrap().as_str(),
        "1",
        "Pair eligibility uses the cached collection observation time"
    );
    crate::test_support::refresh_interval().await;
    let next = prepare(cache).await.unwrap();
    assert_eq!(next.price.value().unwrap().as_str(), "100");
    assert_eq!(next.count, 3);
    runtime.shutdown().await;
}

#[tokio::test]
async fn summary_dependencies_ignore_trait_choices_but_prepared_traits_follow_them() {
    let (_dir, mut writer) = book();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let cache = Arc::new(BookCache::default());
    let scope = BookScope {
        collection: "gift".into(),
        currency: "GRAM".into(),
    };
    let prepare = |cache: Arc<BookCache>, scope: BookScope| {
        source.prepare(move |snapshot| {
            (
                cache.collection(&snapshot, &scope).unwrap().statistics,
                cache.get(&snapshot, &scope).unwrap().statistics,
            )
        })
    };
    let (_, traits) = prepare(cache.clone(), scope.clone()).await.unwrap();
    // Trait preparation publishes the same collection summary; take that cached baseline.
    let (summary, _) = prepare(cache.clone(), scope.clone()).await.unwrap();
    writer
        .apply(w::Batch {
            cursor: "renamed-model".into(),
            changes: vec![entity(w::entity::Entity::AttributeOption(
                w::AttributeOption {
                    id: "m".into(),
                    attribute_id: "tg.model".into(),
                    name: "Renamed model".into(),
                    ..Default::default()
                },
            ))],
        })
        .unwrap();
    crate::test_support::refresh_interval().await;
    let (same, new_traits) = prepare(cache, scope).await.unwrap();
    assert!(Arc::ptr_eq(&summary, &same));
    assert!(!Arc::ptr_eq(&traits, &new_traits));
    runtime.shutdown().await;
}

#[tokio::test]
async fn collection_changes_are_coalesced_until_the_minimum_refresh_interval() {
    let (_dir, mut writer) = book();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let cache = Arc::new(BookCache::default());
    let read = || {
        let cache = cache.clone();
        source.prepare(move |snapshot| {
            cache
                .collection(
                    &snapshot,
                    &BookScope {
                        collection: "gift".into(),
                        currency: "GRAM".into(),
                    },
                )
                .unwrap()
                .statistics
        })
    };
    let first = read().await.unwrap();
    for price in ["2", "1"] {
        writer
            .apply(w::Batch {
                cursor: price.into(),
                changes: vec![ask("busy", "m", "b", price, 999, None)],
            })
            .unwrap();
        assert!(Arc::ptr_eq(&first, &read().await.unwrap()));
    }
    crate::test_support::refresh_interval().await;
    assert!(
        Instant::now()
            >= cache
                .lock()
                .unwrap()
                .books
                .values()
                .next()
                .unwrap()
                .refresh_after
    );
    let next = read().await.unwrap();
    assert!(!Arc::ptr_eq(&first, &next));
    assert_eq!(next.collection.price.as_ref().unwrap().as_str(), "1");
    assert!(Arc::ptr_eq(&next, &read().await.unwrap()));
    runtime.shutdown().await;
}

#[tokio::test]
async fn summary_and_selected_traits_refresh_together_without_a_second_scan() {
    let (_dir, mut writer) = book();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let cache = Arc::new(BookCache::default());
    let scope = BookScope {
        collection: "gift".into(),
        currency: "GRAM".into(),
    };
    let read = |traits: bool| {
        let cache = cache.clone();
        let scope = scope.clone();
        source.prepare(move |snapshot| {
            if traits {
                cache.get(&snapshot, &scope)
            } else {
                cache.collection(&snapshot, &scope)
            }
            .unwrap()
            .statistics
        })
    };
    let before = read(true).await.unwrap();
    writer
        .apply(w::Batch {
            cursor: "changed".into(),
            changes: vec![ask("changed", "m", "b", "1", 999, None)],
        })
        .unwrap();
    crate::test_support::refresh_interval().await;
    let summary = read(false).await.unwrap();
    let traits = read(true).await.unwrap();
    assert!(!Arc::ptr_eq(&before, &traits));
    assert!(!summary.traits_prepared());
    assert!(traits.traits_prepared());
    assert_eq!(summary.evaluated_at, traits.evaluated_at);
    assert_eq!(cache.lock().unwrap().books.len(), 1);
    assert_eq!(summary.collection.price.as_ref().unwrap().as_str(), "1");
    assert_eq!(
        traits.models[&ModelId::from("m")]
            .price
            .as_ref()
            .unwrap()
            .as_str(),
        "1"
    );
    assert!(Arc::ptr_eq(&traits, &read(true).await.unwrap()));
    runtime.shutdown().await;
}

#[tokio::test]
async fn collection_refresh_deadlines_are_independent() {
    let (_dir, mut writer) = book();
    fn other(price: &str) -> w::Change {
        let mut change = ask("other-order", "m", "b", price, 999, None);
        if let Some(w::change::Change::Upsert(w::Entity {
            entity: Some(w::entity::Entity::Order(order)),
        })) = &mut change.change
        {
            order.book.as_mut().unwrap().item_type_id = "other".into();
        }
        change
    }
    writer
        .apply(w::Batch {
            cursor: "two-collections".into(),
            changes: vec![
                entity(w::entity::Entity::ItemType(w::ItemType {
                    id: "other".into(),
                    name: "Other".into(),
                    quantity_step: "1".into(),
                    ..Default::default()
                })),
                other("8"),
            ],
        })
        .unwrap();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let cache = Arc::new(BookCache::default());
    let read = |collection: CollectionId| {
        let cache = cache.clone();
        source.prepare(move |snapshot| {
            cache
                .collection(
                    &snapshot,
                    &BookScope {
                        collection,
                        currency: "GRAM".into(),
                    },
                )
                .unwrap()
                .statistics
        })
    };
    let first = read("gift".into()).await.unwrap();
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    tokio::time::resume();
    let second = read("other".into()).await.unwrap();
    writer
        .apply(w::Batch {
            cursor: "both-changed".into(),
            changes: vec![ask("changed", "m", "b", "1", 888, None), other("4")],
        })
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    tokio::time::resume();
    let updated = read("gift".into()).await.unwrap();
    assert!(!Arc::ptr_eq(&first, &updated));
    assert_eq!(updated.collection.price.as_ref().unwrap().as_str(), "1");
    assert!(Arc::ptr_eq(&second, &read("other".into()).await.unwrap()));
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    tokio::time::resume();
    let other_updated = read("other".into()).await.unwrap();
    assert!(!Arc::ptr_eq(&second, &other_updated));
    assert_eq!(
        other_updated.collection.price.as_ref().unwrap().as_str(),
        "4"
    );
    assert!(Arc::ptr_eq(&updated, &read("gift".into()).await.unwrap()));
    runtime.shutdown().await;
}

#[tokio::test]
async fn summary_and_traits_have_one_owner_and_one_refresh_deadline() {
    let (_dir, mut writer) = book();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let cache = Arc::new(BookCache::default());
    let scope = BookScope {
        collection: "gift".into(),
        currency: "GRAM".into(),
    };
    let read = |traits: bool| {
        let cache = cache.clone();
        let scope = scope.clone();
        source.prepare(move |snapshot| {
            if traits {
                cache.get(&snapshot, &scope)
            } else {
                cache.collection(&snapshot, &scope)
            }
            .unwrap()
        })
    };
    let traits = read(true).await.unwrap();
    let summary = read(false).await.unwrap();
    let deadline = summary.refresh_after;
    assert_eq!(deadline, traits.refresh_after);
    assert_eq!(cache.lock().unwrap().books.len(), 1);
    assert!(Arc::ptr_eq(
        &summary.statistics,
        &cache.lock().unwrap().books[&scope].summary
    ));
    writer
        .apply(w::Batch {
            cursor: "change".into(),
            changes: vec![ask("cheaper", "m", "b", "1", 999, None)],
        })
        .unwrap();
    let same = read(false).await.unwrap();
    assert!(Arc::ptr_eq(&summary.statistics, &same.statistics));
    assert_eq!(same.refresh_after, deadline);
    assert!(Arc::ptr_eq(
        &traits.statistics,
        &read(true).await.unwrap().statistics
    ));
    crate::test_support::refresh_interval().await;
    let refreshed = read(false).await.unwrap();
    assert_eq!(
        refreshed
            .statistics
            .collection
            .price
            .as_ref()
            .unwrap()
            .as_str(),
        "1"
    );
    assert_eq!(
        refreshed.refresh_after,
        read(true).await.unwrap().refresh_after
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn report_validation_does_not_extend_another_consumers_floor_cache_window() {
    let (_dir, mut writer) = book();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let cache = Arc::new(BookCache::default());
    let read = |previous: Option<Arc<crate::market::ValuationRead>>| {
        let cache = cache.clone();
        source.prepare(move |snapshot| {
            let query = crate::ValuationQuery {
                target: crate::ValuationTarget::Selection {
                    collection_id: "gift".into(),
                    criteria: Default::default(),
                },
                currency_id: "GRAM".into(),
                strategies: vec![crate::StrategyId::new("ask_floor").unwrap()],
                period: None,
            }
            .prepare()
            .unwrap();
            crate::market::ValuationRead::read(&snapshot, &query, &cache, previous.as_ref())
                .unwrap()
        })
    };
    let first = read(None).await.unwrap();
    let statistics = first.statistics()[0].clone();
    let initial_deadline = cache
        .lock()
        .unwrap()
        .books
        .values()
        .next()
        .unwrap()
        .refresh_after;
    crate::test_support::refresh_interval().await;
    let validated = read(Some(first.clone())).await.unwrap();
    assert!(Arc::ptr_eq(&first.report, &validated.report));
    assert_eq!(
        cache
            .lock()
            .unwrap()
            .books
            .values()
            .next()
            .unwrap()
            .refresh_after,
        initial_deadline
    );
    assert!(validated.deadline() > Instant::now());
    writer
        .apply(w::Batch {
            cursor: "after-validation".into(),
            changes: vec![ask("cheaper", "m", "b", "1", 999, None)],
        })
        .unwrap();
    let refreshed = read(None).await.unwrap();
    assert!(!Arc::ptr_eq(&statistics, &refreshed.statistics()[0]));
    assert_eq!(
        refreshed.statistics()[0]
            .collection
            .price
            .as_ref()
            .unwrap()
            .as_str(),
        "1"
    );
    runtime.shutdown().await;
}

fn cache_scope(index: usize) -> BookScope {
    BookScope {
        collection: CollectionId::new(format!("cache-{index}")).unwrap(),
        currency: "GRAM".into(),
    }
}
fn cache_collection(index: usize) -> w::Change {
    entity(w::entity::Entity::ItemType(w::ItemType {
        id: cache_scope(index).collection.to_string(),
        name: format!("Collection {index}"),
        quantity_step: "1".into(),
        aliases: vec![w::ExternalId {
            namespace: "tg".into(),
            id: (1000 + index).to_string(),
        }],
        ..Default::default()
    }))
}

#[tokio::test]
async fn captured_deadlines_do_not_change_when_the_cache_is_revalidated() {
    let (_dir, writer) = book();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let cache = Arc::new(BookCache::default());
    let read = || {
        let cache = cache.clone();
        source.prepare(move |snapshot| {
            cache
                .collection(
                    &snapshot,
                    &BookScope {
                        collection: "gift".into(),
                        currency: "GRAM".into(),
                    },
                )
                .unwrap()
        })
    };
    let first = read().await.unwrap();
    let captured = first.refresh_after;
    crate::test_support::refresh_interval().await;
    let validated = read().await.unwrap();
    assert!(Arc::ptr_eq(&first.statistics, &validated.statistics));
    assert!(validated.refresh_after > captured);
    assert_eq!(first.refresh_after, captured);
    assert!(first.refresh_after <= Instant::now());
    assert_eq!(read().await.unwrap().refresh_after, validated.refresh_after);
    runtime.shutdown().await;
}

#[tokio::test]
async fn trait_lru_eviction_keeps_summaries_and_ignores_summary_hits() {
    let (_dir, mut writer) = book();
    writer
        .apply(w::Batch {
            cursor: "trait-cache-capacity".into(),
            changes: (0..=TRAIT_CACHE_LIMIT).map(cache_collection).collect(),
        })
        .unwrap();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    source
        .prepare(|snapshot| {
            let cache = BookCache::default();
            let prepared = (0..TRAIT_CACHE_LIMIT)
                .map(|index| cache.get(&snapshot, &cache_scope(index)).unwrap())
                .collect::<Vec<_>>();
            let first_summary = cache.collection(&snapshot, &cache_scope(0)).unwrap();
            for _ in 0..20 {
                cache.collection(&snapshot, &cache_scope(0)).unwrap();
            }
            cache.get(&snapshot, &cache_scope(1)).unwrap();
            cache
                .get(&snapshot, &cache_scope(TRAIT_CACHE_LIMIT))
                .unwrap();
            {
                let index = cache.lock().unwrap();
                assert_eq!(index.books.len(), TRAIT_CACHE_LIMIT + 1);
                assert!(
                    index.books[&cache_scope(0)].traits.is_none(),
                    "Summary access cannot make unused traits recent"
                );
                assert!(index.books[&cache_scope(1)].traits.is_some());
                assert_eq!(
                    index
                        .books
                        .values()
                        .filter(|entry| entry.traits.is_some())
                        .count(),
                    TRAIT_CACHE_LIMIT
                );
            }
            let retained = cache.collection(&snapshot, &cache_scope(0)).unwrap();
            assert!(Arc::ptr_eq(&first_summary.statistics, &retained.statistics));
            assert_eq!(
                first_summary.refresh_after, retained.refresh_after,
                "Dropping traits cannot reset a summary's deadline"
            );
            let reloaded = cache.get(&snapshot, &cache_scope(0)).unwrap();
            assert!(!Arc::ptr_eq(&prepared[0].statistics, &reloaded.statistics));
            assert!(Arc::ptr_eq(
                &first_summary.statistics,
                &cache
                    .collection(&snapshot, &cache_scope(0))
                    .unwrap()
                    .statistics
            ));
            assert!(
                cache.lock().unwrap().books[&cache_scope(2)]
                    .traits
                    .is_none()
            );
        })
        .await
        .unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn summary_sweeps_preserve_loaded_traits_within_the_total_entry_budget() {
    let (_dir, mut writer) = book();
    writer
        .apply(w::Batch {
            cursor: "summary-cache-capacity".into(),
            changes: (0..=COLLECTION_CACHE_LIMIT).map(cache_collection).collect(),
        })
        .unwrap();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    source
        .prepare(|snapshot| {
            let cache = BookCache::default();
            let selected = BookScope {
                collection: "gift".into(),
                currency: "GRAM".into(),
            };
            let traits = cache.get(&snapshot, &selected).unwrap();
            for index in 0..=COLLECTION_CACHE_LIMIT {
                cache.collection(&snapshot, &cache_scope(index)).unwrap();
                assert!(cache.lock().unwrap().books.len() <= COLLECTION_CACHE_LIMIT);
            }
            let index = cache.lock().unwrap();
            assert_eq!(index.books.len(), COLLECTION_CACHE_LIMIT);
            assert!(!index.books.contains_key(&cache_scope(0)));
            assert!(!index.books.contains_key(&cache_scope(1)));
            assert!(
                index
                    .books
                    .contains_key(&cache_scope(COLLECTION_CACHE_LIMIT))
            );
            assert!(Arc::ptr_eq(
                &traits.statistics,
                &index.books[&selected].traits.as_ref().unwrap().statistics
            ));
            assert_eq!(traits.refresh_after, index.books[&selected].refresh_after);
        })
        .await
        .unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn background_summary_refresh_does_not_promote_trait_recency() {
    let (_dir, mut writer) = book();
    writer
        .apply(w::Batch {
            cursor: "background-recency".into(),
            changes: (0..=TRAIT_CACHE_LIMIT).map(cache_collection).collect(),
        })
        .unwrap();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let cache = Arc::new(BookCache::default());
    let prepare = cache.clone();
    let first = source
        .prepare(move |snapshot| {
            let first = prepare.get(&snapshot, &cache_scope(0)).unwrap().statistics;
            for index in 1..TRAIT_CACHE_LIMIT {
                prepare.get(&snapshot, &cache_scope(index)).unwrap();
            }
            first
        })
        .await
        .unwrap();
    let mut renamed = cache_collection(0);
    if let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::ItemType(item)),
    })) = &mut renamed.change
    {
        item.name = "Renamed collection".into();
    }
    writer
        .apply(w::Batch {
            cursor: "changed-background".into(),
            changes: vec![renamed],
        })
        .unwrap();
    crate::test_support::refresh_interval().await;
    source
        .prepare(move |snapshot| {
            cache.collection(&snapshot, &cache_scope(0)).unwrap();
            assert!(!Arc::ptr_eq(
                &first,
                &cache.lock().unwrap().books[&cache_scope(0)]
                    .traits
                    .as_ref()
                    .unwrap()
                    .statistics
            ));
            cache
                .get(&snapshot, &cache_scope(TRAIT_CACHE_LIMIT))
                .unwrap();
            assert!(
                cache.lock().unwrap().books[&cache_scope(0)]
                    .traits
                    .is_none(),
                "A background rebuild does not count as selecting its traits"
            );
        })
        .await
        .unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn previously_unrequested_pairs_use_the_cached_collection_generation_after_updates() {
    let (_dir, mut writer) = book();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let cache = Arc::new(BookCache::default());
    let prepare = |include_pair: bool| {
        let cache = cache.clone();
        source.prepare(move |snapshot| {
            let currency = "GRAM".into();
            let context = crate::ValuationContext::new(&snapshot, &currency, &cache);
            let stats = context.floor_statistics(&"gift".into()).unwrap();
            let pair = include_pair.then(|| {
                stats
                    .pair_floor(&snapshot, &"m".into(), &"b".into())
                    .unwrap()
            });
            (stats, pair)
        })
    };
    let (initial, _) = prepare(false).await.unwrap();
    writer
        .apply(w::Batch {
            cursor: "changed-pair".into(),
            changes: vec![ask("pair0", "m", "b", "1", 123, None)],
        })
        .unwrap();
    let (cached, pair) = prepare(true).await.unwrap();
    assert!(Arc::ptr_eq(&initial, &cached));
    assert_eq!(
        pair.unwrap().price.value().unwrap().as_str(),
        "100",
        "A newly requested pair must not read newer asks than the cached model/backdrop floors"
    );
    crate::test_support::refresh_interval().await;
    let (fresh, pair) = prepare(true).await.unwrap();
    assert!(!Arc::ptr_eq(&initial, &fresh));
    assert_eq!(fresh.collection.price.as_ref().unwrap().as_str(), "1");
    assert_eq!(pair.unwrap().price.value().unwrap().as_str(), "1");
    let retained_pair = source
        .prepare(move |snapshot| initial.pair_floor(&snapshot, &"m".into(), &"b".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        retained_pair.value().unwrap().as_str(),
        "100",
        "An in-progress calculation retains its floor generation even after cache replacement"
    );
    runtime.shutdown().await;
}
