use super::*;
use crate::{
    gift_floor::tests::{ask, book},
    integration_tests::entity,
    test_support::MarketDataService,
};
use market_hub_protocol::wire as w;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
fn query(strategies: &[&str]) -> ValuationQuery {
    ValuationQuery {
        target: ValuationTarget::Selection {
            collection_id: "gift".into(),
            criteria: market_gifts::GiftCriteria::default(),
        },
        currency_id: "GRAM".into(),
        strategies: strategies
            .iter()
            .map(|id| StrategyId::new(*id).unwrap())
            .collect(),
        period: strategies
            .iter()
            .any(|strategy| strategy.starts_with("execution_"))
            .then(|| market_gifts::Period {
                from: Some("2026-01-01T00:00:00Z".parse().unwrap()),
                until: Some("2026-02-01T00:00:00Z".parse().unwrap()),
            }),
    }
}
#[tokio::test]
async fn identical_consumers_share_reads_without_sharing_their_lifecycle() {
    let (_dir, mut writer) = book();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let request = query(&["execution_mean", "execution_median"]);
    let mut first = handle.watch(request.clone()).await.unwrap();
    let mut second = handle.watch(request.clone()).await.unwrap();
    assert!(Arc::ptr_eq(&first.current(), &second.current()));
    assert!(Arc::ptr_eq(
        &first.current(),
        &handle.query(request.clone()).await.unwrap()
    ));

    // Strategy order is part of the response contract; periods and targets scope observations.
    let mut different = request.clone();
    different.strategies.reverse();
    let reordered = handle.watch(different).await.unwrap();
    assert!(!Arc::ptr_eq(&first.current(), &reordered.current()));
    let mut different = request.clone();
    different.period.as_mut().unwrap().from = Some("2026-01-15T00:00:00Z".parse().unwrap());
    let later = handle.watch(different).await.unwrap();
    assert!(!Arc::ptr_eq(&first.current(), &later.current()));
    let mut different = request;
    different.target = ValuationTarget::Selection {
        collection_id: "gift".into(),
        criteria: market_gifts::GiftCriteria {
            models: vec!["otherm".into()],
            ..Default::default()
        },
    };
    let model = handle.watch(different).await.unwrap();
    assert!(!Arc::ptr_eq(&first.current(), &model.current()));

    writer
        .apply(w::Batch {
            cursor: "shared-history".into(),
            changes: (0..3)
                .map(|i| super::tests::execution(i, "10", "2026-01-10T00:00:00Z"))
                .collect(),
        })
        .unwrap();
    crate::test_support::refresh_interval().await;
    let (a, b) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(first.changed(), second.changed())
    })
    .await
    .unwrap();
    assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
    assert_eq!(
        first.current().results[0]
            .display(&"GRAM".into())
            .fields
            .len(),
        1
    );
    writer.mark(market_replica::SyncState::Stale);
    let (a, b) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(first.changed(), second.changed())
    })
    .await
    .unwrap();
    let stale = a.unwrap();
    assert!(stale.stale);
    assert!(Arc::ptr_eq(&stale, &b.unwrap()));
    drop(first);
    writer.mark(market_replica::SyncState::Live);
    assert!(
        !tokio::time::timeout(Duration::from_secs(2), second.changed())
            .await
            .unwrap()
            .unwrap()
            .stale
    );
    drop((second, reordered, later, model));
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
#[tokio::test]
async fn history_watches_follow_backfills_ignore_asks_and_share_one_scan_per_report() {
    let (_dir, mut writer) = book();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let mut watch = handle
        .watch(query(&["execution_mean", "execution_median", "gift_floor"]))
        .await
        .unwrap();
    assert!(
        matches!(&watch.current().results[2],StrategyOutcome::Error{error,..} if error.code == StrategyFailureCode::UnsupportedTarget)
    );
    writer
        .apply(w::Batch {
            cursor: "ask-only".into(),
            changes: vec![ask("cheap", "m", "b", "1", 900, None)],
        })
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(350), watch.changed())
            .await
            .is_err()
    );
    writer
        .apply(w::Batch {
            cursor: "history".into(),
            changes: (0..3)
                .map(|i| super::tests::execution(i, "10", "2026-01-10T00:00:00Z"))
                .collect(),
        })
        .unwrap();
    crate::test_support::refresh_interval().await;
    let report = tokio::time::timeout(Duration::from_secs(2), watch.changed())
        .await
        .unwrap()
        .unwrap();
    let StrategyOutcome::Value(StrategyValuation::ExecutionMean(mean)) = &report.results[0] else {
        panic!()
    };
    assert_eq!(mean.price.value().unwrap().as_str(), "10");
    let StrategyOutcome::Value(StrategyValuation::ExecutionMedian(median)) = &report.results[1]
    else {
        panic!()
    };
    assert_eq!(median.price.value().unwrap().as_str(), "10");
    writer.mark(market_replica::SyncState::Stale);
    let stale = tokio::time::timeout(Duration::from_secs(2), watch.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(stale.stale);
    assert_eq!(stale.evaluated_at, report.evaluated_at);
    writer.mark(market_replica::SyncState::Live);
    let live = tokio::time::timeout(Duration::from_secs(2), watch.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(!live.stale);
    assert_eq!(live.evaluated_at, report.evaluated_at);
    drop(watch);
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
#[tokio::test]
async fn mixed_strategy_reads_skip_unrelated_changes_and_observe_gift_listing_expiry() {
    let (_dir, mut writer) = book();
    let mut expiring = ask(
        "expiring-gift",
        "m",
        "b",
        "1",
        123,
        Some(jiff::Timestamp::now().as_millisecond() + 1500),
    );
    if let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(order)),
    })) = &mut expiring.change
    {
        order.item_id = Some("gift-pair0".into());
    }
    writer
        .apply(w::Batch {
            cursor: "expiry".into(),
            changes: vec![expiring],
        })
        .unwrap();
    let reads =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = crate::source::ValuationSource::new(reads.handle());
    let cache = Arc::new(crate::floors::BookCache::default());
    let mut request = query(&["ask_floor", "gift_floor"]);
    request.target = ValuationTarget::Gift {
        gift: market_gifts::GiftLookup::Catalog {
            collection_id: "gift".into(),
            number: market_gifts::GiftNumber::new(123).unwrap(),
        },
    };
    let request = request.prepare().unwrap();
    let read = |previous: Option<Arc<ValuationRead>>| {
        let cache = cache.clone();
        let request = request.clone();
        source.prepare(move |snapshot| {
            ValuationRead::read(&snapshot, &request, &cache, previous.as_ref())
        })
    };
    let first = read(None).await.unwrap().unwrap();
    writer
        .apply(w::Batch {
            cursor: "other-currency".into(),
            changes: vec![entity(w::entity::Entity::Currency(w::Currency {
                id: "OTHER".into(),
                name: "Other".into(),
                decimals: 9,
            }))],
        })
        .unwrap();
    let same = read(Some(first.clone())).await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &same));
    assert!(first.deadline() > tokio::time::Instant::now() + Duration::from_secs(58));
    tokio::time::sleep(Duration::from_millis(1550)).await;
    crate::test_support::refresh_interval().await;
    let changed = read(Some(first.clone())).await.unwrap().unwrap();
    assert!(!Arc::ptr_eq(&first, &changed));
    assert!(
        matches!(&changed.report.results[0],StrategyOutcome::Value(StrategyValuation::AskFloor(value)) if value.price.value().unwrap().as_str() == "100")
    );
    reads.shutdown().await;
}

#[tokio::test]
async fn finite_queries_reuse_canonical_reads_and_invalidate_after_market_changes() {
    let (_dir, mut writer) = book();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let mut first = query(&["ask_floor"]);
    first.target = ValuationTarget::Selection {
        collection_id: "gift".into(),
        criteria: market_gifts::GiftCriteria {
            models: vec!["m".into(), "otherm".into(), "m".into()],
            ..Default::default()
        },
    };
    let report = handle.query(first.clone()).await.unwrap();
    let ValuationTarget::Selection { criteria, .. } = &mut first.target else {
        panic!()
    };
    criteria.models = vec!["otherm".into(), "m".into()];
    let reused = handle.query(first.clone()).await.unwrap();
    assert!(Arc::ptr_eq(&report, &reused));
    writer
        .apply(w::Batch {
            cursor: "new-price".into(),
            changes: vec![ask("new-price", "m", "b", "1", 999, None)],
        })
        .unwrap();
    assert!(Arc::ptr_eq(
        &report,
        &handle.query(first.clone()).await.unwrap()
    ));
    crate::test_support::refresh_interval().await;
    let updated = handle.query(first).await.unwrap();
    assert!(!Arc::ptr_eq(&report, &updated));
    let StrategyOutcome::Value(StrategyValuation::AskFloor(value)) = &updated.results[0] else {
        panic!()
    };
    assert_eq!(value.price.value().unwrap().as_str(), "1");
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn narrow_ask_reports_apply_expiry_without_requiring_a_book_change() {
    let (_dir, mut writer) = book();
    let expiry = jiff::Timestamp::now().as_millisecond() + 1000;
    writer
        .apply(w::Batch {
            cursor: "narrow-expiry".into(),
            changes: vec![ask("expiring-pair", "m", "b", "1", 999, Some(expiry))],
        })
        .unwrap();
    let runtime =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = crate::ValuationSource::new(runtime.handle());
    let books = Arc::new(crate::floors::BookCache::default());
    let mut input = query(&["ask_floor"]);
    let ValuationTarget::Selection { criteria, .. } = &mut input.target else {
        unreachable!()
    };
    criteria.models = vec!["m".into()];
    criteria.backdrops = vec!["b".into()];
    let input = input.prepare().unwrap();
    let read = |previous: Option<Arc<ValuationRead>>| {
        let books = books.clone();
        let input = input.clone();
        source.prepare(move |snapshot| {
            ValuationRead::read(&snapshot, &input, &books, previous.as_ref())
        })
    };
    let first = read(None).await.unwrap().unwrap();
    assert!(
        first.statistics().is_empty(),
        "The narrow query does not prepare collection floors"
    );
    assert!(
        matches!(&first.report.results[0], StrategyOutcome::Value(StrategyValuation::AskFloor(value)) if value.price.value().unwrap().as_str() == "1")
    );
    tokio::time::sleep(Duration::from_millis(
        (expiry - jiff::Timestamp::now().as_millisecond()).max(0) as u64 + 20,
    ))
    .await;
    assert!(Arc::ptr_eq(
        &first,
        &read(Some(first.clone())).await.unwrap().unwrap()
    ));
    crate::test_support::refresh_interval().await;
    let next = read(Some(first)).await.unwrap().unwrap();
    assert!(
        matches!(&next.report.results[0], StrategyOutcome::Value(StrategyValuation::AskFloor(value)) if value.price.value().unwrap().as_str() == "100")
    );
    crate::test_support::refresh_interval().await;
    let unchanged = read(Some(next.clone())).await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&next.report, &unchanged.report));
    runtime.shutdown().await;
}

#[tokio::test]
async fn finite_queries_retain_their_previous_result_past_the_refresh_boundary() {
    let (_dir, mut writer) = book();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let request = query(&["execution_mean", "execution_median"]);
    let report = handle.query(request.clone()).await.unwrap();
    let previous = Arc::downgrade(&report);
    drop(report);
    crate::test_support::refresh_interval().await;
    let again = handle.query(request.clone()).await.unwrap();
    assert!(Arc::ptr_eq(
        &again,
        &previous
            .upgrade()
            .expect("Retain data for dependency validation")
    ));
    let changed_at = again.evaluated_at;
    drop(again);
    crate::test_support::refresh_interval().await;
    assert_eq!(
        handle.query(request).await.unwrap().evaluated_at,
        changed_at
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(24 * 60 * 60)).await;
    tokio::time::resume();
    let idle = handle
        .query(query(&["execution_mean", "execution_median"]))
        .await
        .unwrap();
    assert_eq!(
        idle.evaluated_at, changed_at,
        "Idle retention does not force recomputation"
    );
    writer
        .apply(w::Batch {
            cursor: "history-after-idle".into(),
            changes: (0..3)
                .map(|i| super::tests::execution(i, "10", "2026-01-10T00:00:00Z"))
                .collect(),
        })
        .unwrap();
    let request = query(&["execution_mean", "execution_median"]);
    assert!(
        Arc::ptr_eq(&idle, &handle.query(request.clone()).await.unwrap()),
        "Retention preserves the minute gate"
    );
    crate::test_support::refresh_interval().await;
    let refreshed = handle.query(request).await.unwrap();
    assert!(!Arc::ptr_eq(&idle, &refreshed));
    assert!(
        matches!(&refreshed.results[0], StrategyOutcome::Value(StrategyValuation::ExecutionMean(value)) if value.price.value().unwrap().as_str() == "10")
    );
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn active_watches_keep_sharing_after_completed_report_lru_eviction() {
    let (_dir, mut writer) = book();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let request = query(&["ask_floor"]);
    let mut watch = handle.watch(request.clone()).await.unwrap();
    let baseline = watch.current();
    for number in 1..=9 {
        let mut other = request.clone();
        let ValuationTarget::Selection { criteria, .. } = &mut other.target else {
            unreachable!()
        };
        let number = market_gifts::GiftNumber::new(number).unwrap();
        criteria.number = Some(market_gifts::GiftNumberRange {
            min: Some(number),
            max: Some(number),
        });
        handle.query(other).await.unwrap();
    }
    assert!(
        Arc::ptr_eq(&baseline, &handle.query(request.clone()).await.unwrap()),
        "Active watch ownership survives completed-result eviction"
    );
    writer
        .apply(w::Batch {
            cursor: "after-lru-eviction".into(),
            changes: vec![ask("new-floor", "m", "b", "1", 999, None)],
        })
        .unwrap();
    crate::test_support::refresh_interval().await;
    let (watched, queried) = tokio::join!(watch.changed(), handle.query(request));
    let (watched, queried) = (watched.unwrap(), queried.unwrap());
    assert!(Arc::ptr_eq(&watched, &queried));
    assert!(
        matches!(&watched.results[0], StrategyOutcome::Value(StrategyValuation::AskFloor(value)) if value.price.value().unwrap().as_str() == "1")
    );
    drop(watch);
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
