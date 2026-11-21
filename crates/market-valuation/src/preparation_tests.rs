use super::*;
use crate::{
    gift_floor::tests::{ask, book},
    integration_tests::entity,
    source::ValuationSource,
};
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
            let first = cache.collection(&snapshot, &scope).unwrap();
            assert!(!first.traits_prepared());
            assert!(
                first.known_models.is_empty()
                    && first.known_backdrops.is_empty()
                    && first.known_patterns.is_empty()
            );
            assert!(
                first.models.is_empty() && first.backdrops.is_empty() && first.patterns.is_empty()
            );
            assert_eq!(first.collection.evidence().gifts, 12);
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
                &cache.collection(&snapshot, &scope).unwrap()
            ));
            let books = cache.lock().unwrap();
            assert_eq!(books.collections.len(), 21);
            assert!(
                books.traits.is_empty(),
                "Catalog summaries never populate the selected-trait cache"
            );
        })
        .await
        .unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn requested_pairs_are_independent_of_cached_trait_preparation_and_follow_expiry() {
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
            let pair = context
                .pair_floor(&"gift".into(), &"m".into(), &"b".into())
                .unwrap();
            assert!(Arc::ptr_eq(
                &stats,
                &context.floor_statistics(&"gift".into()).unwrap()
            ));
            assert_eq!(cache.lock().unwrap().traits.len(), 1);
            pair
        })
    };
    let first = prepare(cache.clone()).await.unwrap();
    assert_eq!(first.price.value().unwrap().as_str(), "1");
    let remaining = (expiry - jiff::Timestamp::now().as_millisecond()).max(0) as u64;
    tokio::time::sleep(std::time::Duration::from_millis(remaining + 30)).await;
    let next = prepare(cache).await.unwrap();
    assert_eq!(next.price.value().unwrap().as_str(), "100");
    assert_eq!(next.evidence.gifts, 3);
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
                cache.collection(&snapshot, &scope).unwrap(),
                cache.get(&snapshot, &scope).unwrap(),
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
    let (same, new_traits) = prepare(cache, scope).await.unwrap();
    assert!(Arc::ptr_eq(&summary, &same));
    assert!(!Arc::ptr_eq(&traits, &new_traits));
    runtime.shutdown().await;
}
