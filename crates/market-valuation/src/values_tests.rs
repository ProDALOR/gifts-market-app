use super::*;
use crate::{
    gift_floor::tests::{ask, book},
    integration_tests::entity,
    test_support::MarketDataService,
};
use market_hub_protocol::wire as w;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn request() -> MarketValuesRequest {
    MarketValuesRequest {
        currency_id: "GRAM".into(),
        collection_ids: vec!["gift".into()],
        traits_for: Some("gift".into()),
        pair: Some(PairSelection {
            model_id: "m".into(),
            backdrop_id: "b".into(),
        }),
    }
}

#[tokio::test]
async fn batch_reuses_gift_statistics_and_status_changes_do_not_rebuild_price_data() {
    let (_dir, mut writer) = book();
    let reads =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = source::ValuationSource::new(reads.handle());
    let cache = Arc::new(BookCache::default());
    let prepare = |previous: Option<Arc<ValuesRead>>| {
        let cache = cache.clone();
        source.prepare(move |snapshot| {
            ValuesRead::read(&snapshot, &request(), &cache, previous.as_ref())
        })
    };
    let first = prepare(None).await.unwrap().unwrap();
    assert_eq!(
        first.values.collections[0]
            .floor
            .price
            .value()
            .unwrap()
            .as_str(),
        "3"
    );
    let traits = first.values.traits.as_ref().unwrap();
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
    assert!(stale.values.stale);
    assert!(Arc::ptr_eq(&first.stats[0], &stale.stats[0]));
    writer.mark(market_replica::SyncState::Live);
    let live = prepare(Some(stale)).await.unwrap().unwrap();
    assert!(!live.values.stale);
    assert!(Arc::ptr_eq(&first.stats[0], &live.stats[0]));
    let missing = source
        .prepare(move |snapshot| {
            let mut query = request();
            query.pair.as_mut().unwrap().model_id = "removed".into();
            ValuesRead::read(&snapshot, &query, &cache, None)
        })
        .await
        .unwrap()
        .unwrap();
    assert!(missing.values.collections[0].floor.price.value().is_some());
    assert_eq!(
        serde_json::to_value(
            &missing
                .values
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
    let mut watch = handle.watch_values(request()).await.unwrap();
    assert_eq!(
        watch.current().collections[0]
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
        next.collections[0].floor.price.value().unwrap().as_str(),
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
        tokio::time::timeout(Duration::from_secs(2), watch.changed())
            .await
            .unwrap()
            .unwrap()
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
        watches.push(handle.watch_values(request()).await.unwrap());
    }
    assert_eq!(
        handle.watch_values(request()).await.err().unwrap().code(),
        ValuationErrorCode::CapacityExceeded
    );
    watches.pop();
    watches.push(handle.watch_values(request()).await.unwrap());
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
    query.collection_ids.push("gift".into());
    assert!(query.validate().is_err());
    query.collection_ids = vec!["other".into()];
    assert!(query.validate().is_err());
    query.traits_for = None;
    assert!(query.validate().is_err());
    query.pair = None;
    assert!(query.validate().is_ok());
    query.collection_ids = (0..9)
        .map(|n| CollectionId::new(format!("gift{n}")).unwrap())
        .collect();
    assert!(query.validate().is_err());
}
