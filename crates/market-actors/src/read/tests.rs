use super::*;
use crate::{
    MarketDataService,
    tests::{entity, fixture, order},
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
        .dependencies([], &CurrencyId::from("GRAM"), CatalogTrait::Model, &[])
        .unwrap();
    let held = data
        .reader
        .budget
        .slots
        .clone()
        .acquire_owned()
        .await
        .unwrap();
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
    let held = data
        .reader
        .budget
        .slots
        .clone()
        .acquire_owned()
        .await
        .unwrap();
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
    assert_eq!(data.reader.workers.available_permits(), 0);
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
