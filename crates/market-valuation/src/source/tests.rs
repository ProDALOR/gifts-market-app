use super::*;
use crate::{
    integration_tests::{entity, fixture, order},
    test_support::MarketDataService,
};
use market_gifts::CurrencyId;
use market_hub_protocol::wire as w;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
#[tokio::test]
async fn dependency_checks_use_revisions_then_a_bounded_worker_and_memoize_success() {
    let (_dir, mut writer) = fixture();
    let data = MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let deps = data
        .reader
        .snapshot()
        .unwrap()
        .dependencies([], &CurrencyId::from("GRAM"), &[CatalogTrait::Model], &[])
        .unwrap();
    let held = data.reader.executor.budget().wait().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), data.reader.is_current(&deps))
            .await
            .unwrap()
    );
    writer
        .apply(w::Batch {
            cursor: "metadata".into(),
            changes: vec![entity(w::entity::Entity::Currency(w::Currency {
                id: "XTR".into(),
                name: "Stars".into(),
                decimals: 0,
            }))],
        })
        .unwrap();
    let mut validation = Box::pin(data.reader.is_current(&deps));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut validation)
            .await
            .is_err()
    );
    assert!(data.handle.status().await.unwrap().ready);
    drop(held);
    assert!(validation.await);
    let held = data.reader.executor.budget().wait().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), data.reader.is_current(&deps))
            .await
            .unwrap()
    );
    drop(held);
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
#[tokio::test]
async fn ask_traversal_continues_after_an_empty_eligible_page_and_stays_pinned() {
    let (_dir, mut writer) = fixture();
    let changes = (0..1000)
        .map(|i| {
            let mut row = order("3");
            if let Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::Order(order)),
            })) = &mut row.change
            {
                order.id = format!("ask.{i:04}");
                order.expires_at_ms = (i < 500).then_some(10);
            }
            row
        })
        .collect();
    writer
        .apply(w::Batch {
            cursor: "asks".into(),
            changes,
        })
        .unwrap();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 1, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(runtime.handle());
    let snapshot = source.snapshot().unwrap();
    let scope = BookScope {
        collection: "gift".into(),
        currency: "GRAM".into(),
    };
    let first = snapshot.asks(&scope, 10, String::new()).unwrap();
    assert!(first.data.is_empty());
    assert!(!first.next_page_token.is_empty());
    // Updating the writer during traversal must not replace the pinned order.
    writer
        .apply(w::Batch {
            cursor: "changed".into(),
            changes: vec![order("99")],
        })
        .unwrap();
    let mut token = first.next_page_token;
    let mut count = 0;
    let mut pages = 1;
    loop {
        let page = snapshot.asks(&scope, 10, token).unwrap();
        pages += 1;
        count += page.data.len();
        assert!(page.data.iter().all(|quote| quote.price.as_str() == "3"));
        if page.next_page_token.is_empty() {
            break;
        }
        token = page.next_page_token;
        assert!(
            pages <= 3,
            "Traversal must advance across equal-price orders"
        );
    }
    assert_eq!(count, 501);
    assert_eq!(pages, 3);
    runtime.shutdown().await;
}

#[tokio::test]
async fn cancelling_a_worker_stops_its_scan_at_the_next_page_and_releases_capacity() {
    let (_dir, mut writer) = fixture();
    let changes = (0..601)
        .map(|i| {
            let mut row = order("3");
            if let Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::Order(order)),
            })) = &mut row.change
            {
                order.id = format!("ask.{i:04}");
            }
            row
        })
        .collect();
    writer
        .apply(w::Batch {
            cursor: "pages".into(),
            changes,
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    let (finished, done) = tokio::sync::oneshot::channel();
    let reader = data.reader.clone();
    let task = tokio::spawn(async move {
        reader
            .prepare(move |snapshot| {
                let scope = BookScope {
                    collection: "gift".into(),
                    currency: "GRAM".into(),
                };
                let page = snapshot.asks(&scope, 0, String::new()).unwrap();
                assert!(!page.next_page_token.is_empty());
                let _ = entered.send(page.data.len());
                let _ = proceed.recv();
                let result = snapshot.asks(&scope, 0, page.next_page_token);
                let _ = finished.send(result.is_err());
            })
            .await
    });
    assert_eq!(started.await.unwrap(), 500);
    task.abort();
    let _ = task.await;
    assert!(
        tokio::time::timeout(Duration::from_millis(20), data.reader.prepare(|_| ()))
            .await
            .is_err()
    );
    release.send(()).unwrap();
    assert!(done.await.unwrap());
    tokio::time::timeout(Duration::from_secs(1), data.reader.prepare(|_| ()))
        .await
        .unwrap()
        .unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn valuation_and_data_actor_queries_share_one_admission_budget() {
    use market_gifts::{GiftFilters, GiftOrderQuery};
    let (_dir, writer) = fixture();
    let data = MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let held = data.reader.executor.budget().try_query().unwrap();
    let error = data
        .handle
        .query_orders(GiftOrderQuery::new(GiftFilters::new(
            "gift".into(),
            "GRAM".into(),
        )))
        .await
        .unwrap_err();
    assert_eq!(
        error.code(),
        market_actors::error::DataErrorCode::CapacityExceeded
    );
    let mut work = Box::pin(data.reader.prepare(|snapshot| snapshot.revision()));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut work)
            .await
            .is_err()
    );
    assert!(data.handle.status().await.unwrap().ready);
    drop(held);
    assert_eq!(work.await.unwrap().unwrap(), "c0");
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[cfg(feature = "portfolio")]
#[tokio::test]
async fn exact_collection_lookup_ignores_a_directory_larger_than_the_catalog_response_budget() {
    use market_gifts::ModelLookup;
    let (_dir, mut writer) = fixture();
    writer
        .apply(w::Batch {
            cursor: "large-directory".into(),
            changes: (0..50_001)
                .map(|i| {
                    entity(w::entity::Entity::ItemType(w::ItemType {
                        id: format!("unrelated.{i}"),
                        name: "Unrelated".into(),
                        quantity_step: "1".into(),
                        ..Default::default()
                    }))
                })
                .collect(),
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let resolved = data
        .reader
        .prepare(|snapshot| {
            snapshot.resolve_models(vec![ModelLookup::Collection {
                collection: "gift".into(),
            }])
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved[0].collection.as_ref().unwrap().as_str(), "gift");
    assert!(resolved[0].unresolved.is_none());
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
