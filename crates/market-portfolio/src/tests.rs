use super::*;
use market_invocations::{
    AttemptError, Invocation, Policy, RateKey, RateLimit, Replay, Retry,
    Runtime as InvocationRuntime, operation,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;
struct Call {
    request: InventoryPageRequest,
    reply: Option<oneshot::Sender<Result<InventoryPage, InventoryError>>>,
}
#[derive(Clone, Default)]
struct Source {
    calls: Arc<Mutex<Vec<Call>>>,
}
fn inventory_policy(timeout: Duration) -> Policy {
    Policy {
        limits: vec![RateLimit {
            key: RateKey {
                owner: 42,
                group: "gifts",
                resource: None,
            },
            interval: Duration::from_millis(1),
            concurrency: 1,
        }],
        replay: Replay::Safe,
        attempt_timeout: timeout,
        max_attempts: 1,
    }
}
impl InventoryProvider for Source {
    fn page(
        &self,
        request: InventoryPageRequest,
    ) -> impl Invocation<Output = InventoryPage, Error = InventoryError> {
        let source = self.clone();
        operation(inventory_policy(Duration::from_secs(1)), move || {
            let source = source.clone();
            let request = request.clone();
            async move {
                let (reply, response) = oneshot::channel();
                source.calls.lock().unwrap().push(Call {
                    request,
                    reply: Some(reply),
                });
                response
                    .await
                    .unwrap_or(Err(InventoryError::Unavailable))
                    .map_err(|error| {
                        let retry = match &error {
                            InventoryError::RetryAfter(delay) => Retry::After {
                                delay: *delay,
                                scope: inventory_policy(Duration::from_secs(1)).limits[0]
                                    .key
                                    .clone(),
                            },
                            _ => Retry::Never,
                        };
                        AttemptError { error, retry }
                    })
            }
        })
    }
}
impl Source {
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    fn request(&self, n: usize) -> InventoryPageRequest {
        self.calls.lock().unwrap()[n].request.clone()
    }
    fn reply(&self, n: usize, page: Result<InventoryPage, InventoryError>) {
        self.calls.lock().unwrap()[n]
            .reply
            .take()
            .unwrap()
            .send(page)
            .unwrap();
    }
}
#[derive(Clone, Default)]
struct Quotes(Arc<AtomicUsize>, Arc<Mutex<Vec<Arc<Vec<InventoryGift>>>>>);
impl ValuationProvider for Quotes {
    async fn value(
        &self,
        gifts: Arc<Vec<InventoryGift>>,
        currency: market_gifts::CurrencyId,
        progress: tokio::sync::watch::Sender<usize>,
    ) -> Result<Arc<InventoryValuation>, market_valuation::ValuationError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        self.1.lock().unwrap().push(gifts.clone());
        progress.send_replace(gifts.len());
        Ok(Arc::new(InventoryValuation {
            currency_id: currency,
            evaluated_at: jiff::Timestamp::now(),
            stale: false,
            total: Some((gifts.len() * 2).to_string().try_into().unwrap()),
            priced: gifts.len(),
            unpriced: 0,
            items: gifts
                .iter()
                .cloned()
                .map(|gift| market_valuation::InventoryValue {
                    gift,
                    resolved: None,
                    estimate: Some("2".into()),
                    error: None,
                })
                .collect(),
        }))
    }
}
pub(crate) fn gift(id: &str) -> InventoryGift {
    InventoryGift {
        gift: market_gifts::TelegramGiftDetails {
            id: id.to_owned().try_into().unwrap(),
            slug: format!("Gift-{id}").try_into().unwrap(),
            number: id.parse::<u32>().unwrap().try_into().unwrap(),
            collection: market_gifts::TelegramGiftId::new(1).unwrap(),
            model: market_gifts::TelegramDocumentId::new(2).unwrap(),
            backdrop: Some(3),
            pattern: Some(market_gifts::TelegramDocumentId::new(4).unwrap()),
        },
        relation: GiftRelation::Owned,
    }
}
pub(crate) fn page(ids: &[&str], next: Option<&str>, total: usize) -> InventoryPage {
    InventoryPage {
        gifts: ids.iter().map(|id| gift(id)).collect(),
        scanned: ids.len(),
        next_cursor: next.map(str::to_owned),
        reported_total: Some(total),
        ..Default::default()
    }
}
fn query(user: u64, currency: &str) -> PortfolioQuery {
    PortfolioQuery {
        user_id: UserId::new(user).unwrap(),
        currency_id: currency.to_owned().try_into().unwrap(),
    }
}
async fn calls(source: &Source, n: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while source.count() < n {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}
async fn stage(watch: &mut PortfolioWatch, expected: PortfolioStage) -> Arc<PortfolioState> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let state = watch.current();
            if state.progress.stage == expected {
                return state;
            }
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}
async fn service(config: PortfolioConfig) -> (PortfolioService, Source, Quotes, InvocationRuntime) {
    let source = Source::default();
    let quotes = Quotes::default();
    let runtime = InvocationRuntime::start(Default::default()).unwrap();
    let service = PortfolioService::start(source.clone(), quotes.clone(), config, runtime.handle())
        .await
        .unwrap();
    (service, source, quotes, runtime)
}
#[test]
fn inventory_preserves_profile_position_across_pages_and_duplicates() {
    let mut builder = crate::inventory::InventoryBuilder::default();
    let config = PortfolioConfig::default();
    builder
        .merge(page(&["90", "3", "25"], Some("next"), 5), 100, &config)
        .unwrap();
    builder
        .merge(page(&["3", "1"], None, 5), 100, &config)
        .unwrap();
    let inventory = builder.snapshot(InventoryCompletion::Complete);
    assert_eq!(
        inventory
            .gifts
            .iter()
            .map(|gift| gift.gift.id.as_str())
            .collect::<Vec<_>>(),
        vec!["90", "3", "25", "1"]
    );
}
#[tokio::test]
async fn bot_and_gui_share_paginated_refresh_and_currency_changes_do_not_fetch() {
    let (service, source, quotes, calls_runtime) = service(Default::default()).await;
    let cached = service.handle.query(query(7, "GRAM")).await.unwrap();
    assert_eq!(cached.progress.stage, PortfolioStage::Idle);
    assert_eq!(source.count(), 0);
    let mut bot = service.handle.watch(query(7, "GRAM")).await.unwrap();
    let mut gui = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    assert_eq!(source.request(0).cursor, None);
    service.handle.refresh(query(7, "GRAM")).await.unwrap();
    assert_eq!(source.count(), 1);
    let mut first_page = page(&["1", "2"], Some("next"), 4);
    first_page.gifts[1].relation = GiftRelation::Unverified;
    source.reply(0, Ok(first_page));
    calls(&source, 2).await;
    assert_eq!(source.request(1).cursor.as_deref(), Some("next"));
    let progress = stage(&mut gui, PortfolioStage::LoadingInventory).await;
    assert_eq!(progress.progress.checked, 2);
    assert_eq!(progress.progress.total, Some(4));
    source.reply(1, Ok(page(&["2", "3"], None, 4)));
    let result = stage(&mut bot, PortfolioStage::Ready).await;
    assert_eq!(result.inventory.as_ref().unwrap().count, 3);
    assert_eq!(result.inventory.as_ref().unwrap().skipped, 0);
    assert_eq!(
        result.valuation.as_ref().unwrap().items[1].gift.relation,
        GiftRelation::Unverified
    );
    assert_eq!(
        result
            .valuation
            .as_ref()
            .unwrap()
            .total
            .as_ref()
            .unwrap()
            .to_string(),
        "6"
    );
    assert_eq!(
        stage(&mut gui, PortfolioStage::Ready).await.refresh_id,
        result.refresh_id
    );
    let mut stars = service.handle.watch(query(7, "XTR")).await.unwrap();
    let result = stage(&mut stars, PortfolioStage::Ready).await;
    assert_eq!(
        result.valuation.as_ref().unwrap().currency_id.as_str(),
        "XTR"
    );
    assert_eq!(source.count(), 2);
    assert_eq!(quotes.0.load(Ordering::SeqCst), 2);
    let inputs = quotes.1.lock().unwrap();
    assert!(
        Arc::ptr_eq(&inputs[0], &inputs[1]),
        "currency views share one inventory allocation"
    );
    drop(inputs);
    assert!(matches!(
        service.handle.refresh(query(7, "GRAM")).await,
        Err(PortfolioError::Cooldown(_))
    ));
    service.task.shutdown().await.unwrap();
    calls_runtime.shutdown().await;
}
#[tokio::test]
async fn failed_or_partial_recheck_keeps_the_last_complete_report() {
    let config = PortfolioConfig {
        refresh_interval: Duration::from_millis(15),
        ..Default::default()
    };
    let (service, source, _, runtime) = service(config).await;
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["1", "2"], None, 2)));
    let first = stage(&mut watch, PortfolioStage::Ready).await;
    for (i, result) in [Err(InventoryError::Unavailable), Ok(page(&["3"], None, 20))]
        .into_iter()
        .enumerate()
    {
        tokio::time::sleep(Duration::from_millis(25)).await;
        service.handle.refresh(query(7, "GRAM")).await.unwrap();
        calls(&source, i + 2).await;
        source.reply(i + 1, result);
        let failed = stage(&mut watch, PortfolioStage::Failed).await;
        assert_eq!(
            failed.inventory.as_ref().unwrap().checked_at,
            first.inventory.as_ref().unwrap().checked_at
        );
        assert!(Arc::ptr_eq(
            failed.valuation.as_ref().unwrap(),
            first.valuation.as_ref().unwrap()
        ));
    }
    let mut reopened = service.handle.watch(query(7, "GRAM")).await.unwrap();
    assert_eq!(reopened.current().progress.stage, PortfolioStage::Failed);
    assert_eq!(source.count(), 3);
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}
#[tokio::test]
async fn active_watch_is_not_evicted_and_shutdown_wakes_observers() {
    let (service, source, _, runtime) = service(PortfolioConfig {
        cached_users: 1,
        ..Default::default()
    })
    .await;
    let mut first = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&[], None, 0)));
    stage(&mut first, PortfolioStage::Ready).await;
    assert!(matches!(
        service.handle.query(query(8, "GRAM")).await,
        Err(PortfolioError::Busy)
    ));
    service.task.shutdown().await.unwrap();
    assert!(matches!(
        first.changed().await,
        Err(PortfolioError::Stopped)
    ));
    runtime.shutdown().await;
}
#[test]
fn inventory_builder_bounds_cursors_and_accounts_for_every_item() {
    let config = PortfolioConfig::default();
    let mut builder = crate::inventory::InventoryBuilder::default();
    assert!(
        builder
            .merge(page(&["1"], Some("same"), 3), 100, &config)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        builder
            .merge(page(&["1"], Some("same"), 3), 100, &config)
            .unwrap(),
        Some(InventoryCompletion::Partial(PartialReason::RepeatedCursor))
    );
    let mut malformed = page(&["2"], None, 1);
    malformed.scanned = 2;
    assert!(
        crate::inventory::InventoryBuilder::default()
            .merge(malformed, 100, &config)
            .is_err()
    );
    let partial = crate::inventory::InventoryBuilder::default()
        .merge(page(&["1"], None, 2), 100, &config)
        .unwrap();
    assert_eq!(
        partial,
        Some(InventoryCompletion::Partial(PartialReason::MissingCursor))
    );
    assert!(UserId::new(0).is_err());
    assert!(UserId::new(u64::MAX).is_err());
}

#[derive(Clone, Default)]
struct DelayedQuotes(Arc<Mutex<Vec<oneshot::Sender<()>>>>);
impl ValuationProvider for DelayedQuotes {
    async fn value(
        &self,
        gifts: Arc<Vec<InventoryGift>>,
        currency: market_gifts::CurrencyId,
        progress: tokio::sync::watch::Sender<usize>,
    ) -> Result<Arc<InventoryValuation>, market_valuation::ValuationError> {
        let (tx, rx) = oneshot::channel();
        self.0.lock().unwrap().push(tx);
        let _ = rx.await;
        Quotes::default().value(gifts, currency, progress).await
    }
}
async fn quote_calls(quotes: &DelayedQuotes, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while quotes.0.lock().unwrap().len() < count {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn inventory_and_prices_replace_atomically_and_old_generation_is_cancelled() {
    let source = Source::default();
    let quotes = DelayedQuotes::default();
    let runtime = InvocationRuntime::start(Default::default()).unwrap();
    let service = PortfolioService::start(
        source.clone(),
        quotes.clone(),
        PortfolioConfig {
            refresh_interval: Duration::from_millis(10),
            ..Default::default()
        },
        runtime.handle(),
    )
    .await
    .unwrap();
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["1"], None, 1)));
    quote_calls(&quotes, 1).await;
    assert!(watch.current().inventory.is_none());
    quotes.0.lock().unwrap().remove(0).send(()).unwrap();
    let first = stage(&mut watch, PortfolioStage::Ready).await;
    tokio::time::sleep(Duration::from_millis(15)).await;
    service.handle.refresh(query(7, "GRAM")).await.unwrap();
    calls(&source, 2).await;
    source.reply(1, Ok(page(&["2", "3"], None, 2)));
    quote_calls(&quotes, 1).await;
    let pending = stage(&mut watch, PortfolioStage::Valuing).await;
    assert_eq!(pending.inventory.as_ref().unwrap().count, 1);
    assert!(Arc::ptr_eq(
        pending.valuation.as_ref().unwrap(),
        first.valuation.as_ref().unwrap()
    ));
    tokio::time::sleep(Duration::from_millis(15)).await;
    service.handle.refresh(query(7, "GRAM")).await.unwrap();
    calls(&source, 3).await;
    assert!(
        quotes.0.lock().unwrap()[0].is_closed(),
        "recheck cancels superseded valuation before downloading inventory"
    );
    source.reply(2, Ok(page(&["4", "5", "6"], None, 3)));
    quote_calls(&quotes, 2).await;
    assert!(
        quotes.0.lock().unwrap().remove(0).send(()).is_err(),
        "superseded valuation is dropped"
    );
    quotes.0.lock().unwrap().remove(0).send(()).unwrap();
    let ready = stage(&mut watch, PortfolioStage::Ready).await;
    assert_eq!(ready.inventory.as_ref().unwrap().count, 3);
    assert_eq!(
        ready.valuation.as_ref().unwrap().items[0]
            .gift
            .gift
            .id
            .as_str(),
        "4"
    );
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}
#[tokio::test]
async fn shutdown_drops_an_inventory_page_in_flight() {
    let (service, source, _, runtime) = service(Default::default()).await;
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    service.task.shutdown().await.unwrap();
    assert!(matches!(
        watch.changed().await,
        Err(PortfolioError::Stopped)
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        while !source.calls.lock().unwrap()[0]
            .reply
            .as_ref()
            .unwrap()
            .is_closed()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn cached_queries_do_not_emit_duplicate_progress_or_move_the_cooldown_deadline() {
    let (service, source, _, runtime) = service(Default::default()).await;
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["1"], None, 1)));
    let ready = stage(&mut watch, PortfolioStage::Ready).await;
    let cached = service.handle.query(query(7, "GRAM")).await.unwrap();
    assert_eq!(cached.refresh_after, ready.refresh_after);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), watch.changed())
            .await
            .is_err()
    );
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}

#[derive(Clone, Default)]
struct ProgressQuotes(Arc<Mutex<Vec<(tokio::sync::watch::Sender<usize>, oneshot::Sender<()>)>>>);
impl ValuationProvider for ProgressQuotes {
    async fn value(
        &self,
        gifts: Arc<Vec<InventoryGift>>,
        currency: market_gifts::CurrencyId,
        progress: tokio::sync::watch::Sender<usize>,
    ) -> Result<Arc<InventoryValuation>, market_valuation::ValuationError> {
        let (finish, wait) = oneshot::channel();
        self.0.lock().unwrap().push((progress.clone(), finish));
        let _ = wait.await;
        Quotes::default().value(gifts, currency, progress).await
    }
}
async fn progress_service(
    idle: Duration,
) -> (PortfolioService, Source, ProgressQuotes, InvocationRuntime) {
    let source = Source::default();
    let quotes = ProgressQuotes::default();
    let runtime = InvocationRuntime::start(Default::default()).unwrap();
    let service = PortfolioService::start(
        source.clone(),
        quotes.clone(),
        PortfolioConfig {
            valuation_idle_timeout: idle,
            refresh_interval: Duration::from_millis(1),
            ..Default::default()
        },
        runtime.handle(),
    )
    .await
    .unwrap();
    (service, source, quotes, runtime)
}
async fn progress_calls(quotes: &ProgressQuotes, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while quotes.0.lock().unwrap().len() < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn valuation_keeps_running_beyond_the_idle_interval_while_progress_advances() {
    let (service, source, quotes, runtime) = progress_service(Duration::from_millis(300)).await;
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["1", "2", "3", "4"], None, 4)));
    progress_calls(&quotes, 1).await;
    let started = std::time::Instant::now();
    for count in 1..=3 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        quotes.0.lock().unwrap()[0].0.send_replace(count);
        // Public updates are sampled less often than this fixture's idle timeout.
        // Internal progress must keep the run alive even when no frame is emitted.
        assert_eq!(watch.current().progress.stage, PortfolioStage::Valuing);
    }
    assert!(started.elapsed() > Duration::from_millis(300));
    quotes.0.lock().unwrap().remove(0).1.send(()).unwrap();
    let ready = stage(&mut watch, PortfolioStage::Ready).await;
    assert_eq!(ready.valuation.as_ref().unwrap().priced, 4);
    assert_eq!(
        ready.progress.valued, 4,
        "completion bypasses progress sampling"
    );
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}
#[tokio::test]
async fn valuation_progress_coalesces_steps_and_completion_is_immediate() {
    let (service, source, quotes, runtime) = progress_service(Duration::from_secs(2)).await;
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["1", "2", "3", "4", "5", "6"], None, 6)));
    progress_calls(&quotes, 1).await;
    stage(&mut watch, PortfolioStage::Valuing).await;
    for count in 1..=4 {
        quotes.0.lock().unwrap()[0].0.send_replace(count);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), watch.changed())
            .await
            .is_err(),
        "fast steps do not emit separate public updates"
    );
    let progress = tokio::time::timeout(Duration::from_secs(1), watch.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        progress.progress.valued, 4,
        "the sampled update contains the latest count"
    );
    quotes.0.lock().unwrap().remove(0).1.send(()).unwrap();
    let ready = tokio::time::timeout(
        Duration::from_millis(250),
        stage(&mut watch, PortfolioStage::Ready),
    )
    .await
    .unwrap();
    assert_eq!(
        ready.progress.valued, 6,
        "completion does not wait for the next sample"
    );
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn stalled_valuation_cancels_work_and_retains_the_previous_report() {
    let (service, source, quotes, runtime) = progress_service(Duration::from_millis(100)).await;
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["1"], None, 1)));
    progress_calls(&quotes, 1).await;
    quotes.0.lock().unwrap().remove(0).1.send(()).unwrap();
    let first = stage(&mut watch, PortfolioStage::Ready).await;
    tokio::time::sleep(Duration::from_millis(2)).await;
    service.handle.refresh(query(7, "GRAM")).await.unwrap();
    calls(&source, 2).await;
    source.reply(1, Ok(page(&["1", "2"], None, 2)));
    progress_calls(&quotes, 1).await;
    // Repeated identical counts must not keep a stalled job alive.
    let sender = quotes.0.lock().unwrap()[0].0.clone();
    let repeat = tokio::spawn(async move {
        loop {
            sender.send_replace(0);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let failed = stage(&mut watch, PortfolioStage::Failed).await;
    repeat.abort();
    assert!(failed.error.as_deref().unwrap().contains("no progress"));
    assert!(Arc::ptr_eq(
        failed.valuation.as_ref().unwrap(),
        first.valuation.as_ref().unwrap()
    ));
    assert_eq!(failed.inventory, first.inventory);
    assert!(
        quotes.0.lock().unwrap()[0].1.is_closed(),
        "stalled provider future is dropped"
    );
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn item_versions_ignore_check_times_but_change_with_profile_order() {
    let (service, source, _, runtime) = service(PortfolioConfig {
        refresh_interval: Duration::from_millis(1),
        ..Default::default()
    })
    .await;
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["90", "3", "25"], None, 3)));
    let first = stage(&mut watch, PortfolioStage::Ready).await;
    tokio::time::sleep(Duration::from_millis(3)).await;
    service.handle.refresh(query(7, "GRAM")).await.unwrap();
    calls(&source, 2).await;
    source.reply(1, Ok(page(&["90", "3", "25"], None, 3)));
    let same = stage(&mut watch, PortfolioStage::Ready).await;
    assert_eq!(
        first.version, same.version,
        "unchanged revaluation needs no page downloads"
    );
    assert_ne!(
        first.inventory.as_ref().unwrap().checked_at,
        same.inventory.as_ref().unwrap().checked_at
    );
    tokio::time::sleep(Duration::from_millis(3)).await;
    service.handle.refresh(query(7, "GRAM")).await.unwrap();
    calls(&source, 3).await;
    source.reply(2, Ok(page(&["25", "90", "3"], None, 3)));
    let changed = stage(&mut watch, PortfolioStage::Ready).await;
    assert_ne!(same.version, changed.version);
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn check_cooldown_starts_after_completion_including_failures() {
    let interval = Duration::from_millis(200);
    assert_eq!(
        PortfolioConfig::default().refresh_interval,
        Duration::from_secs(300)
    );
    let (service, source, _, runtime) = service(PortfolioConfig {
        refresh_interval: interval,
        ..Default::default()
    })
    .await;
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    assert!(
        watch.current().refresh_after.is_none(),
        "running checks have no completion deadline yet"
    );
    tokio::time::sleep(interval + Duration::from_millis(30)).await;
    service.handle.refresh(query(7, "GRAM")).await.unwrap();
    assert_eq!(
        source.count(),
        1,
        "requests join the check even after its starting deadline would have passed"
    );
    source.reply(0, Ok(page(&["1"], None, 1)));
    let first = stage(&mut watch, PortfolioStage::Ready).await;
    assert!(first.refresh_after.unwrap() > first.inventory.as_ref().unwrap().checked_at);
    assert!(
        matches!(
            service.handle.refresh(query(7, "XTR")).await,
            Err(PortfolioError::Cooldown(_))
        ),
        "currency changes cannot bypass a just-completed check"
    );
    assert_eq!(source.count(), 1);
    tokio::time::sleep(interval + Duration::from_millis(20)).await;
    service.handle.refresh(query(7, "GRAM")).await.unwrap();
    calls(&source, 2).await;
    source.reply(1, Err(InventoryError::Unavailable));
    let failed = stage(&mut watch, PortfolioStage::Failed).await;
    assert!(failed.refresh_after > first.refresh_after);
    assert!(
        matches!(
            service.handle.refresh(query(7, "GRAM")).await,
            Err(PortfolioError::Cooldown(_))
        ),
        "failed checks are also throttled"
    );
    assert!(Arc::ptr_eq(
        failed.valuation.as_ref().unwrap(),
        first.valuation.as_ref().unwrap()
    ));
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn completed_valuation_cooldown_survives_new_inventory_and_cached_requests() {
    let interval = Duration::from_millis(200);
    let source = Source::default();
    let quotes = DelayedQuotes::default();
    let runtime = InvocationRuntime::start(Default::default()).unwrap();
    let service = PortfolioService::start(
        source.clone(),
        quotes.clone(),
        PortfolioConfig {
            refresh_interval: interval,
            ..Default::default()
        },
        runtime.handle(),
    )
    .await
    .unwrap();
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["1"], None, 1)));
    quote_calls(&quotes, 1).await;
    // A run longer than the interval must still get a full cooldown after finishing.
    tokio::time::sleep(interval + Duration::from_millis(30)).await;
    for _ in 0..3 {
        service.handle.query(query(7, "GRAM")).await.unwrap();
    }
    assert_eq!(
        quotes.0.lock().unwrap().len(),
        1,
        "in-flight valuation is shared"
    );
    quotes.0.lock().unwrap().remove(0).send(()).unwrap();
    let first = stage(&mut watch, PortfolioStage::Ready).await;
    // The inventory is independently eligible, but its new generation cannot reset valuation pacing.
    service.handle.refresh(query(7, "GRAM")).await.unwrap();
    calls(&source, 2).await;
    source.reply(1, Ok(page(&["2", "3"], None, 2)));
    let mut reopened = service.handle.watch(query(7, "GRAM")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    for _ in 0..3 {
        let cached = service.handle.query(query(7, "GRAM")).await.unwrap();
        assert!(Arc::ptr_eq(
            cached.valuation.as_ref().unwrap(),
            first.valuation.as_ref().unwrap()
        ));
        assert_eq!(cached.inventory.as_ref().unwrap().count, 1);
    }
    assert!(
        quotes.0.lock().unwrap().is_empty(),
        "new inventory must wait for the previous valuation cooldown"
    );
    tokio::time::sleep(interval).await;
    service.handle.query(query(7, "GRAM")).await.unwrap();
    quote_calls(&quotes, 1).await;
    quotes.0.lock().unwrap().remove(0).send(()).unwrap();
    let updated = stage(&mut reopened, PortfolioStage::Ready).await;
    assert_eq!(updated.inventory.as_ref().unwrap().count, 2);
    assert_eq!(
        updated.valuation.as_ref().unwrap().items[0]
            .gift
            .gift
            .id
            .as_str(),
        "2"
    );
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn cache_eviction_does_not_reset_recent_portfolio_cooldowns() {
    let interval = Duration::from_millis(200);
    let (service, source, quotes, runtime) = service(PortfolioConfig {
        cached_users: 1,
        refresh_interval: interval,
        ..Default::default()
    })
    .await;
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["1"], None, 1)));
    stage(&mut watch, PortfolioStage::Ready).await;
    drop(watch);
    assert!(matches!(
        service.handle.query(query(8, "GRAM")).await,
        Err(PortfolioError::Busy)
    ));
    let mut reopened = service.handle.watch(query(7, "GRAM")).await.unwrap();
    assert_eq!(reopened.current().progress.stage, PortfolioStage::Ready);
    assert_eq!(source.count(), 1);
    assert_eq!(quotes.0.load(Ordering::SeqCst), 1);
    drop(reopened);
    tokio::time::sleep(interval + Duration::from_millis(20)).await;
    service.handle.query(query(8, "GRAM")).await.unwrap();
    assert_eq!(
        source.count(),
        1,
        "cached-only queries never fetch inventory"
    );
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}

#[derive(Clone, Default)]
struct FailedQuotes(Arc<AtomicUsize>);
impl ValuationProvider for FailedQuotes {
    async fn value(
        &self,
        _: Arc<Vec<InventoryGift>>,
        _: market_gifts::CurrencyId,
        _: tokio::sync::watch::Sender<usize>,
    ) -> Result<Arc<InventoryValuation>, market_valuation::ValuationError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(market_valuation::ValuationError::timed_out("test failure"))
    }
}
#[tokio::test]
async fn failed_valuations_wait_before_retrying() {
    let source = Source::default();
    let quotes = FailedQuotes::default();
    let runtime = InvocationRuntime::start(Default::default()).unwrap();
    let interval = Duration::from_millis(200);
    let service = PortfolioService::start(
        source.clone(),
        quotes.clone(),
        PortfolioConfig {
            refresh_interval: interval,
            ..Default::default()
        },
        runtime.handle(),
    )
    .await
    .unwrap();
    let mut watch = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["1"], None, 1)));
    stage(&mut watch, PortfolioStage::Failed).await;
    for _ in 0..3 {
        service.handle.query(query(7, "GRAM")).await.unwrap();
    }
    assert_eq!(quotes.0.load(Ordering::SeqCst), 1);
    tokio::time::sleep(interval + Duration::from_millis(20)).await;
    service.handle.query(query(7, "GRAM")).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while quotes.0.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(source.count(), 1);
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn switching_currencies_cannot_evict_a_cooling_valuation() {
    let (service, source, quotes, runtime) = service(Default::default()).await;
    let mut first = service.handle.watch(query(7, "GRAM")).await.unwrap();
    calls(&source, 1).await;
    source.reply(0, Ok(page(&["1"], None, 1)));
    let ready = stage(&mut first, PortfolioStage::Ready).await;
    drop(first);
    for i in 0..7 {
        let mut view = service
            .handle
            .watch(query(7, &format!("C{i}")))
            .await
            .unwrap();
        stage(&mut view, PortfolioStage::Ready).await;
    }
    assert!(matches!(
        service.handle.watch(query(7, "EXTRA")).await,
        Err(PortfolioError::Busy)
    ));
    let cached = service.handle.query(query(7, "GRAM")).await.unwrap();
    assert!(Arc::ptr_eq(
        cached.valuation.as_ref().unwrap(),
        ready.valuation.as_ref().unwrap()
    ));
    assert_eq!(source.count(), 1);
    assert_eq!(quotes.0.load(Ordering::SeqCst), 8);
    service.task.shutdown().await.unwrap();
    runtime.shutdown().await;
}
