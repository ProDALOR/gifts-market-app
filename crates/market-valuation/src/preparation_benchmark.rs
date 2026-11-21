//! Reproducible CPU comparison using real replica reads; no hub or Telegram connection.
use crate::{
    gift_floor::tests::{ask, book},
    integration_tests::entity,
    source::{BookScope, ValuationSource},
};
use market_hub_protocol::wire as w;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
#[tokio::test]
#[ignore = "manual preparation benchmark"]
async fn benchmark_preparation() {
    let (_dir, mut writer) = book();
    let models = (0..100)
        .map(|i| format!("bench-model-{i}"))
        .collect::<Vec<_>>();
    let backdrops = (0..100)
        .map(|i| format!("bench-backdrop-{i}"))
        .collect::<Vec<_>>();
    let mut changes = vec![];
    for (attribute, ids) in [("tg.model", &models), ("tg.backdrop", &backdrops)] {
        for id in ids {
            changes.push(entity(w::entity::Entity::AttributeOption(
                w::AttributeOption {
                    id: id.clone(),
                    attribute_id: attribute.into(),
                    name: id.clone(),
                    ..Default::default()
                },
            )));
        }
    }
    changes.push(entity(w::entity::Entity::ItemType(w::ItemType {
        id: "gift".into(),
        name: "Gift".into(),
        quantity_step: "1".into(),
        attributes: [
            (
                "tg.model".into(),
                w::AttributeRule {
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: models.clone(),
                    }),
                    ..Default::default()
                },
            ),
            (
                "tg.backdrop".into(),
                w::AttributeRule {
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: backdrops.clone(),
                    }),
                    ..Default::default()
                },
            ),
            ("tg.number".into(), w::AttributeRule::default()),
        ]
        .into(),
        ..Default::default()
    })));
    for i in 0..20_000 {
        changes.push(ask(
            &format!("bench-{i}"),
            &models[i % 100],
            &backdrops[(i / 100) % 100],
            "100",
            i as i64 + 1000,
            None,
        ));
    }
    writer
        .apply(w::Batch {
            cursor: "benchmark".into(),
            changes,
        })
        .unwrap();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    for (label, preparation) in [
        ("collection", crate::floors::Preparation::Collection),
        ("independent_traits", crate::floors::Preparation::Traits),
    ] {
        let mut elapsed = vec![];
        for _ in 0..5 {
            let time = Instant::now();
            let stats = source
                .prepare(move |snapshot| {
                    crate::floors::Statistics::read(
                        &snapshot,
                        &BookScope {
                            collection: "gift".into(),
                            currency: "GRAM".into(),
                        },
                        preparation,
                    )
                })
                .await
                .unwrap()
                .unwrap();
            elapsed.push(time.elapsed());
            if preparation == crate::floors::Preparation::Collection {
                assert!(stats.known_models.is_empty());
                assert!(stats.models.is_empty());
                assert!(stats.backdrops.is_empty());
            }
            assert_eq!(stats.collection.evidence().listings, 20_012);
        }
        elapsed.sort();
        println!(
            "{label}: median={:?}; 20,012 asks; zero precomputed pairs",
            elapsed[2]
        );
    }
    let mut elapsed = vec![];
    for _ in 0..5 {
        let time = Instant::now();
        let pair = source
            .prepare(|snapshot| {
                let books = crate::floors::BookCache::default();
                let currency = "GRAM".into();
                let context = crate::ValuationContext::new(&snapshot, &currency, &books);
                context.pair_floor(
                    &"gift".into(),
                    &"bench-model-0".into(),
                    &"bench-backdrop-0".into(),
                )
            })
            .await
            .unwrap()
            .unwrap();
        elapsed.push(time.elapsed());
        assert_eq!(pair.evidence.gifts, 2);
    }
    elapsed.sort();
    println!("requested_pair: median={:?}; 2 matching asks", elapsed[2]);
    source
        .prepare(|snapshot| {
            let cache = crate::floors::BookCache::default();
            let scope = BookScope {
                collection: "gift".into(),
                currency: "GRAM".into(),
            };
            let first = cache.collection(&snapshot, &scope).unwrap().statistics;
            let time = Instant::now();
            for _ in 0..1_000 {
                assert!(std::sync::Arc::ptr_eq(
                    &first,
                    &cache.collection(&snapshot, &scope).unwrap().statistics
                ));
            }
            println!(
                "cached_collection: average={:?}; unchanged book, no traversal",
                time.elapsed() / 1_000
            );
        })
        .await
        .unwrap();
    runtime.shutdown().await;
}
