use super::*;
use market_gifts::{SyncState, SyncStatus, TelegramDocumentId, TelegramGiftId};
use market_invocations::{
    AttemptError, Invocation, Policy, RateKey, RateLimit, Replay, Retry,
    Runtime as InvocationRuntime, operation,
};
use market_valuation::ValuationError;
use market_valuation::{ModelValue, Valuation, ValuationRequest};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::oneshot, time::Instant};
use tokio_util::sync::CancellationToken;

struct Call {
    request: InventoryPageRequest,
    at: Instant,
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
            interval: Duration::from_millis(300),
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
                    at: Instant::now(),
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
    fn cancelled(&self, n: usize) -> bool {
        self.calls.lock().unwrap()[n]
            .reply
            .as_ref()
            .is_none_or(|reply| reply.is_closed())
    }
}
#[derive(Clone, Default)]
struct Quotes {
    calls: Arc<Mutex<Vec<ValuationRequest>>>,
    errors: Arc<Mutex<VecDeque<ValuationError>>>,
}
impl ValuationProvider for Quotes {
    async fn mark(
        &self,
        request: ValuationRequest,
    ) -> Result<Arc<market_valuation::Mark<Valuation>>, ValuationError> {
        let mut calls = self.calls.lock().unwrap();
        calls.push(request.clone());
        if let Some(error) = self.errors.lock().unwrap().pop_front() {
            return Err(error);
        }
        let count: u32 = request.positions.iter().map(|p| p.quantity).sum();
        let value = Valuation {
            rows: request
                .positions
                .iter()
                .map(|p| ModelValue {
                    identity: None,
                    collection: "Collection".into(),
                    model: "Model".into(),
                    quantity: p.quantity,
                    floor: Some("2".into()),
                    value: Some((p.quantity * 2).to_string().try_into().unwrap()),
                    venue_id: None,
                    unpriced: None,
                })
                .collect(),
            total: (count * 2).to_string().try_into().unwrap(),
            priced: count,
            unpriced: 0,
            currency_id: request.currency_id,
            sync: SyncStatus {
                ready: true,
                state: SyncState::Live,
                updated_at: None,
            },
        };
        Ok(Arc::new(market_valuation::Mark::evaluated(
            "model_floor",
            market_valuation::Prepared {
                prepared_at: jiff::Timestamp::now(),
                revision: format!("quote-{}", calls.len()),
                data: value,
            },
        )))
    }
}
fn user(id: u64) -> UserId {
    UserId::new(id).unwrap()
}
fn gift(id: &str) -> InventoryGift {
    InventoryGift {
        id: id.to_owned().try_into().unwrap(),
        collection_id: TelegramGiftId::new(1).unwrap(),
        model_document_id: TelegramDocumentId::new(2).unwrap(),
        relation: GiftRelation::Owned,
    }
}
fn page(ids: &[&str], next: Option<&str>, total: usize) -> InventoryPage {
    InventoryPage {
        gifts: ids.iter().map(|id| gift(id)).collect(),
        scanned: ids.len(),
        next_cursor: next.map(str::to_owned),
        reported_total: Some(total),
        ..Default::default()
    }
}
fn request(id: u64, currency: &str, priority: Priority) -> PortfolioRequest {
    PortfolioRequest {
        strategy: market_valuation::StrategyId::model_floor(),
        user: user(id),
        currency_id: currency.to_owned().try_into().unwrap(),
        priority,
    }
}
fn setup(
    source: Source,
    quotes: Quotes,
    config: PortfolioConfig,
) -> (
    PortfolioHandle,
    CancellationToken,
    CancellationToken,
    InvocationRuntime,
) {
    config.validate().unwrap();
    let stop = CancellationToken::new();
    let closed = CancellationToken::new();
    let invocations = InvocationRuntime::start(Default::default()).unwrap();
    let handle = crate::actor::start(
        source,
        quotes,
        config,
        invocations.handle(),
        stop.clone(),
        closed.clone(),
    );
    (handle, stop, closed, invocations)
}

fn ask(
    handle: &PortfolioHandle,
    id: u64,
    currency: &str,
    priority: Priority,
) -> tokio::task::JoinHandle<Result<Portfolio, PortfolioError>> {
    let handle = handle.clone();
    let request = request(id, currency, priority);
    tokio::spawn(async move { handle.request(request).await })
}
async fn spin(mut ready: impl FnMut() -> bool) {
    for _ in 0..2000 {
        if ready() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("actor did not make expected progress");
}
async fn flush() {
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
}
async fn next(source: &Source, n: usize) {
    spin(|| source.count() >= n).await;
}
async fn shutdown(stop: CancellationToken, closed: CancellationToken) {
    stop.cancel();
    closed.cancelled().await;
}

#[actix::test]
async fn coalesces_pages_and_currencies_deduplicates_gifts_and_revalues_cached_inventory() {
    tokio::time::pause();
    let source = Source::default();
    let quotes = Quotes::default();
    let (handle, stop, closed, _invocations) =
        setup(source.clone(), quotes.clone(), PortfolioConfig::default());
    let a = ask(&handle, 1, "GRAM", Priority::Normal);
    let b = ask(&handle, 1, "XTR", Priority::Interactive);
    next(&source, 1).await;
    assert_eq!(source.request(0).limit, 100);
    source.reply(0, Ok(page(&["one", "two"], Some("page-2"), 4)));
    flush().await;
    assert_eq!(source.count(), 1);
    tokio::time::advance(Duration::from_millis(301)).await;
    next(&source, 2).await;
    assert_eq!(source.request(1).cursor.as_deref(), Some("page-2"));
    let mut last = page(&["two", "three"], None, 4);
    last.gifts[1].relation = GiftRelation::Hosted;
    source.reply(1, Ok(last));
    let a = a.await.unwrap().unwrap();
    let b = b.await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&a.inventory, &b.inventory));
    assert_eq!(a.inventory.gifts.len(), 3);
    assert_eq!(a.inventory.gifts_scanned, 4);
    assert_eq!(a.inventory.pages_read, 2);
    assert_eq!(a.inventory.completion, InventoryCompletion::Complete);
    assert!(
        a.inventory
            .gifts
            .iter()
            .any(|gift| gift.relation == GiftRelation::Hosted)
    );
    assert_eq!(a.valuation.total.as_str(), "6");
    assert_eq!(quotes.calls.lock().unwrap().len(), 2);
    assert!(
        quotes
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.positions.len() == 1 && r.positions[0].quantity == 3)
    );
    let fresh = handle
        .request(request(1, "GRAM", Priority::Normal))
        .await
        .unwrap();
    assert_eq!(source.count(), 2);
    assert_ne!(fresh.valuation.revision, a.valuation.revision);
    assert!(Arc::ptr_eq(&fresh.inventory, &a.inventory));
    shutdown(stop, closed).await;
}

#[actix::test]
async fn priority_is_fifo_per_class_and_background_gets_a_turn() {
    tokio::time::pause();
    let source = Source::default();
    let (handle, stop, closed, _invocations) = setup(
        source.clone(),
        Quotes::default(),
        PortfolioConfig::default(),
    );
    let first = ask(&handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    let background = ask(&handle, 2, "GRAM", Priority::Background);
    flush().await;
    let mut interactive = vec![];
    for id in 3..=9 {
        interactive.push(ask(&handle, id, "GRAM", Priority::Interactive));
        flush().await;
    }
    source.reply(0, Ok(page(&["gift"], None, 1)));
    flush().await;
    let mut order = vec![];
    for n in 1..=8 {
        tokio::time::advance(Duration::from_millis(301)).await;
        next(&source, n + 1).await;
        order.push(source.request(n).user.get());
        source.reply(n, Ok(page(&["gift"], None, 1)));
        flush().await;
    }
    assert!(
        order.iter().position(|id| *id == 2).unwrap() < 7,
        "background must not starve: {order:?}"
    );
    assert_eq!(
        order.into_iter().filter(|id| *id != 2).collect::<Vec<_>>(),
        (3..=9).collect::<Vec<_>>()
    );
    first.await.unwrap().unwrap();
    background.await.unwrap().unwrap();
    for result in interactive {
        result.await.unwrap().unwrap();
    }
    shutdown(stop, closed).await;
}

#[actix::test]
async fn multi_page_inventory_yields_and_duplicate_waiters_promote_queued_work() {
    tokio::time::pause();
    let source = Source::default();
    let (handle, stop, closed, _invocations) = setup(
        source.clone(),
        Quotes::default(),
        PortfolioConfig::default(),
    );
    let first = ask(&handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    let low = ask(&handle, 2, "GRAM", Priority::Background);
    flush().await;
    let other = ask(&handle, 3, "GRAM", Priority::Interactive);
    flush().await;
    let promoted = ask(&handle, 2, "XTR", Priority::Interactive);
    flush().await;
    source.reply(0, Ok(page(&["a"], Some("next"), 2)));
    flush().await;
    for (n, id) in [(1, 3), (2, 2), (3, 1)] {
        tokio::time::advance(Duration::from_millis(301)).await;
        next(&source, n + 1).await;
        assert_eq!(source.request(n).user, user(id));
        source.reply(n, Ok(page(&["b"], None, if id == 1 { 2 } else { 1 })));
        flush().await;
    }
    for task in [first, low, other, promoted] {
        task.await.unwrap().unwrap();
    }
    assert_eq!(source.count(), 4);
    shutdown(stop, closed).await;
}

#[actix::test]
async fn flood_wait_is_global_survives_cancellation_and_does_not_block_cached_valuation() {
    tokio::time::pause();
    let source = Source::default();
    let (handle, stop, closed, _invocations) = setup(
        source.clone(),
        Quotes::default(),
        PortfolioConfig::default(),
    );
    let cached = ask(&handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    source.reply(0, Ok(page(&["g"], None, 1)));
    cached.await.unwrap().unwrap();
    tokio::time::advance(Duration::from_millis(301)).await;
    let waiting = ask(&handle, 2, "GRAM", Priority::Normal);
    next(&source, 2).await;
    source.reply(1, Err(InventoryError::RetryAfter(Duration::from_secs(20))));
    flush().await;
    waiting.abort();
    let _ = waiting.await;
    flush().await;
    let other = ask(&handle, 3, "GRAM", Priority::Interactive);
    flush().await;
    let cached = handle
        .request(request(1, "XTR", Priority::Normal))
        .await
        .unwrap();
    assert_eq!(cached.valuation.currency_id.as_str(), "XTR");
    tokio::time::advance(Duration::from_secs(19)).await;
    flush().await;
    assert_eq!(source.count(), 2);
    tokio::time::advance(Duration::from_millis(1001)).await;
    next(&source, 3).await;
    assert_eq!(source.request(2).user, user(3));
    {
        let calls = source.calls.lock().unwrap();
        assert!(calls[2].at.duration_since(calls[1].at) >= Duration::from_secs(20));
    }
    source.reply(2, Ok(page(&[], None, 0)));
    other.await.unwrap().unwrap();
    shutdown(stop, closed).await;
}

#[actix::test]
async fn cancellation_is_per_waiter_and_capacity_is_bounded() {
    tokio::time::pause();
    let source = Source::default();
    let config = PortfolioConfig {
        max_requests: 2,
        ..Default::default()
    };
    let (handle, stop, closed, _invocations) = setup(source.clone(), Quotes::default(), config);
    let a = ask(&handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    let b = ask(&handle, 1, "GRAM", Priority::Normal);
    flush().await;
    assert!(matches!(
        handle
            .request(request(2, "GRAM", Priority::Interactive))
            .await,
        Err(PortfolioError::Busy)
    ));
    a.abort();
    let _ = a.await;
    flush().await;
    assert!(!source.cancelled(0));
    source.reply(0, Ok(page(&["g"], None, 1)));
    b.await.unwrap().unwrap();
    tokio::time::advance(Duration::from_millis(301)).await;
    let a = ask(&handle, 2, "GRAM", Priority::Normal);
    next(&source, 2).await;
    a.abort();
    let _ = a.await;
    spin(|| source.cancelled(1)).await;
    let b = ask(&handle, 3, "GRAM", Priority::Normal);
    tokio::time::advance(Duration::from_millis(301)).await;
    next(&source, 3).await;
    source.reply(2, Ok(page(&[], None, 0)));
    b.await.unwrap().unwrap();
    shutdown(stop, closed).await;
}

#[actix::test]
async fn repeated_and_missing_cursors_and_later_failures_are_explicit_and_not_cached() {
    tokio::time::pause();
    let source = Source::default();
    let (handle, stop, closed, _invocations) = setup(
        source.clone(),
        Quotes::default(),
        PortfolioConfig::default(),
    );
    for (id, final_page, reason) in [
        (
            1,
            Ok(page(&["b"], Some("next"), 3)),
            PartialReason::RepeatedCursor,
        ),
        (2, Ok(page(&[], None, 3)), PartialReason::MissingCursor),
        (
            3,
            Err(InventoryError::Unavailable),
            PartialReason::ReadFailed(InventoryError::Unavailable),
        ),
    ] {
        let start = source.count();
        let result = ask(&handle, id, "GRAM", Priority::Normal);
        tokio::time::advance(Duration::from_millis(301)).await;
        next(&source, start + 1).await;
        source.reply(start, Ok(page(&["a"], Some("next"), 3)));
        flush().await;
        tokio::time::advance(Duration::from_millis(301)).await;
        next(&source, start + 2).await;
        source.reply(start + 1, final_page);
        let result = result.await.unwrap().unwrap();
        assert_eq!(
            result.inventory.completion,
            InventoryCompletion::Partial(reason)
        );
        let retry = ask(&handle, id, "GRAM", Priority::Normal);
        tokio::time::advance(Duration::from_millis(301)).await;
        next(&source, start + 3).await;
        assert_eq!(source.request(start + 2).cursor, None);
        retry.abort();
        let _ = retry.await;
        flush().await;
    }
    shutdown(stop, closed).await;
}

#[actix::test]
async fn provider_errors_never_become_empty_portfolios_and_bad_pages_are_rejected() {
    tokio::time::pause();
    let source = Source::default();
    let quotes = Quotes::default();
    let (handle, stop, closed, _invocations) =
        setup(source.clone(), quotes.clone(), PortfolioConfig::default());
    for error in [
        InventoryError::Forbidden,
        InventoryError::Authorization,
        InventoryError::Unavailable,
    ] {
        let n = source.count();
        let result = ask(&handle, 1, "GRAM", Priority::Normal);
        tokio::time::advance(Duration::from_millis(301)).await;
        next(&source, n + 1).await;
        source.reply(n, Err(error.clone()));
        assert!(
            matches!(result.await.unwrap(), Err(PortfolioError::Inventory(actual)) if actual == error)
        );
    }
    assert!(quotes.calls.lock().unwrap().is_empty());
    let n = source.count();
    let result = ask(&handle, 1, "GRAM", Priority::Normal);
    tokio::time::advance(Duration::from_millis(301)).await;
    next(&source, n + 1).await;
    source.reply(
        n,
        Ok(InventoryPage {
            scanned: 101,
            ..Default::default()
        }),
    );
    assert!(matches!(
        result.await.unwrap(),
        Err(PortfolioError::Inventory(InventoryError::InvalidData(_)))
    ));
    let n = source.count();
    let result = ask(&handle, 1, "GRAM", Priority::Normal);
    tokio::time::advance(Duration::from_millis(301)).await;
    next(&source, n + 1).await;
    source.reply(
        n,
        Ok(InventoryPage {
            scanned: 1,
            reported_total: Some(1),
            ..Default::default()
        }),
    );
    assert!(matches!(
        result.await.unwrap(),
        Err(PortfolioError::Inventory(InventoryError::InvalidData(_)))
    ));
    assert!(
        quotes.calls.lock().unwrap().is_empty(),
        "unaccounted gifts must never produce a complete empty valuation"
    );
    shutdown(stop, closed).await;
}

#[actix::test]
async fn cache_expires_evicts_lru_and_does_not_cache_valuation_errors() {
    tokio::time::pause();
    let source = Source::default();
    let quotes = Quotes::default();
    let config = PortfolioConfig {
        cached_users: 2,
        inventory_ttl: Duration::from_secs(5),
        ..Default::default()
    };
    let (handle, stop, closed, _invocations) = setup(source.clone(), quotes.clone(), config);
    for id in [1, 2] {
        let n = source.count();
        let result = ask(&handle, id, "GRAM", Priority::Normal);
        tokio::time::advance(Duration::from_millis(301)).await;
        next(&source, n + 1).await;
        source.reply(n, Ok(page(&["g"], None, 1)));
        result.await.unwrap().unwrap();
    }
    handle
        .request(request(1, "GRAM", Priority::Normal))
        .await
        .unwrap(); // user 2 is least recent
    let third = ask(&handle, 3, "GRAM", Priority::Normal);
    tokio::time::advance(Duration::from_millis(301)).await;
    next(&source, 3).await;
    source.reply(2, Ok(page(&["g"], None, 1)));
    third.await.unwrap().unwrap();
    let evicted = ask(&handle, 2, "GRAM", Priority::Normal);
    tokio::time::advance(Duration::from_millis(301)).await;
    next(&source, 4).await;
    source.reply(3, Ok(page(&["g"], None, 1)));
    evicted.await.unwrap().unwrap();
    quotes
        .errors
        .lock()
        .unwrap()
        .push_back(ValuationError::unavailable("temporary read failure"));
    assert!(matches!(
        handle.request(request(2, "GRAM", Priority::Normal)).await,
        Err(PortfolioError::Valuation(_))
    ));
    assert!(
        handle
            .request(request(2, "GRAM", Priority::Normal))
            .await
            .is_ok()
    );
    assert_eq!(source.count(), 4);
    tokio::time::advance(Duration::from_secs(5)).await;
    let expired = ask(&handle, 2, "GRAM", Priority::Normal);
    next(&source, 5).await;
    source.reply(4, Ok(page(&[], None, 0)));
    expired.await.unwrap().unwrap();
    shutdown(stop, closed).await;
}

#[actix::test]
async fn page_and_request_timeouts_cancel_work_without_poisoning_the_actor() {
    tokio::time::pause();
    let source = Source::default();
    let config = PortfolioConfig {
        request_timeout: Duration::from_secs(3),
        valuation_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let (handle, stop, closed, _invocations) = setup(source.clone(), Quotes::default(), config);
    let stalled = ask(&handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        stalled.await.unwrap(),
        Err(PortfolioError::Inventory(InventoryError::Unavailable))
    ));
    assert!(source.cancelled(0));
    let stalled = ask(&handle, 2, "GRAM", Priority::Normal);
    next(&source, 2).await;
    source.reply(1, Err(InventoryError::RetryAfter(Duration::from_secs(20))));
    flush().await;
    tokio::time::advance(Duration::from_secs(3)).await;
    assert!(matches!(
        stalled.await.unwrap(),
        Err(PortfolioError::TimedOut)
    ));
    flush().await;
    shutdown(stop, closed).await;
}

#[tokio::test]
async fn service_works_outside_actix_and_shutdown_closes_all_handles_and_provider_work() {
    let invocations = InvocationRuntime::start(Default::default()).unwrap();
    let source = Source::default();
    let service = PortfolioService::start(
        source.clone(),
        Quotes::default(),
        PortfolioConfig::default(),
        invocations.handle(),
    )
    .await
    .unwrap();
    let result = ask(&service.handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    service.task.shutdown().await.unwrap();
    assert!(matches!(
        result.await.unwrap(),
        Err(PortfolioError::Stopped)
    ));
    assert!(source.cancelled(0));
    assert!(matches!(
        service
            .handle
            .request(request(1, "GRAM", Priority::Normal))
            .await,
        Err(PortfolioError::Stopped)
    ));
}
#[tokio::test]
async fn invalid_config_and_identity_fail_without_starting_a_worker() {
    let invocations = InvocationRuntime::start(Default::default()).unwrap();
    assert!(UserId::new(0).is_err());
    assert!(UserId::new(u64::MAX).is_err());
    let config = PortfolioConfig {
        max_requests: usize::MAX,
        ..Default::default()
    };
    assert!(matches!(
        PortfolioService::start(
            Source::default(),
            Quotes::default(),
            config,
            invocations.handle()
        )
        .await,
        Err(PortfolioError::InvalidConfig(_))
    ));
}

#[actix::test]
async fn reads_seven_hundred_collectibles_with_counts_and_appearance_identity_intact() {
    tokio::time::pause();
    let source = Source::default();
    let quotes = Quotes::default();
    let (handle, stop, closed, _invocations) =
        setup(source.clone(), quotes.clone(), PortfolioConfig::default());
    let result = ask(&handle, 1, "GRAM", Priority::Interactive);
    for (n, offset) in (0..748).step_by(100).enumerate() {
        if n > 0 {
            tokio::time::advance(Duration::from_millis(301)).await;
        }
        next(&source, n + 1).await;
        let end = (offset + 100).min(748);
        let mut response = InventoryPage {
            scanned: end - offset,
            reported_total: Some(748),
            next_cursor: (end < 748).then(|| end.to_string()),
            ..Default::default()
        };
        for id in offset..end {
            if id < 45 {
                response.regular_gifts += 1;
            } else if id < 48 {
                *response
                    .skipped
                    .entry(SkipReason::MissingModel)
                    .or_default() += 1;
            } else {
                let mut gift = gift(&id.to_string());
                if id % 2 == 0 {
                    gift.model_document_id = TelegramDocumentId::new(3).unwrap();
                }
                gift.relation = if id == 48 {
                    GiftRelation::Hosted
                } else if id == 49 {
                    GiftRelation::Unverified
                } else {
                    GiftRelation::Owned
                };
                response.gifts.push(gift);
            }
        }
        source.reply(n, Ok(response));
        flush().await;
    }
    let result = result.await.unwrap().unwrap();
    assert_eq!(result.inventory.pages_read, 8);
    assert_eq!(result.inventory.gifts.len(), 700);
    assert_eq!(result.inventory.gifts_scanned, 748);
    assert_eq!(result.inventory.regular_gifts, 45);
    assert_eq!(result.inventory.skipped[&SkipReason::MissingModel], 3);
    assert_eq!(result.inventory.completion, InventoryCompletion::Complete);
    assert_eq!(result.valuation.priced, 700);
    assert_eq!(result.valuation.total.as_str(), "1400");
    {
        let calls = quotes.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].positions.len(), 2);
        assert!(
            calls[0]
                .positions
                .iter()
                .all(|position| position.quantity == 350)
        );
    }
    shutdown(stop, closed).await;
}

#[actix::test]
async fn gift_and_page_limits_return_explicit_partial_snapshots_and_bound_provider_requests() {
    tokio::time::pause();
    let source = Source::default();
    let config = PortfolioConfig {
        page_size: 3,
        max_gifts: 5,
        ..Default::default()
    };
    let (handle, stop, closed, _invocations) = setup(source.clone(), Quotes::default(), config);
    let result = ask(&handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    assert_eq!(source.request(0).limit, 3);
    source.reply(0, Ok(page(&["a", "b", "c"], Some("p2"), 6)));
    flush().await;
    tokio::time::advance(Duration::from_millis(301)).await;
    next(&source, 2).await;
    assert_eq!(source.request(1).limit, 2);
    source.reply(1, Ok(page(&["d", "e"], Some("p3"), 6)));
    let result = result.await.unwrap().unwrap();
    assert_eq!(result.inventory.gifts.len(), 5);
    assert_eq!(
        result.inventory.completion,
        InventoryCompletion::Partial(PartialReason::LimitReached)
    );
    shutdown(stop, closed).await;
    let source = Source::default();
    let config = PortfolioConfig {
        max_pages: 1,
        ..Default::default()
    };
    let (handle, stop, closed, _invocations) = setup(source.clone(), Quotes::default(), config);
    let result = ask(&handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    source.reply(0, Ok(page(&[], Some("no-progress"), 1)));
    assert_eq!(
        result.await.unwrap().unwrap().inventory.completion,
        InventoryCompletion::Partial(PartialReason::LimitReached)
    );
    assert_eq!(source.count(), 1);
    shutdown(stop, closed).await;
}

#[actix::test]
async fn full_flood_wait_is_not_truncated_when_all_original_requests_time_out() {
    tokio::time::pause();
    let source = Source::default();
    let (handle, stop, closed, _invocations) = setup(
        source.clone(),
        Quotes::default(),
        PortfolioConfig::default(),
    );
    let result = ask(&handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    source.reply(
        0,
        Err(InventoryError::RetryAfter(Duration::from_secs(7200))),
    );
    flush().await;
    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(matches!(
        result.await.unwrap(),
        Err(PortfolioError::TimedOut)
    ));
    flush().await;
    tokio::time::advance(Duration::from_secs(7139)).await;
    let result = ask(&handle, 2, "GRAM", Priority::Interactive);
    flush().await;
    assert_eq!(source.count(), 1);
    tokio::time::advance(Duration::from_millis(1001)).await;
    next(&source, 2).await;
    source.reply(1, Ok(page(&[], None, 0)));
    result.await.unwrap().unwrap();
    shutdown(stop, closed).await;
}

#[actix::test]
async fn coalesces_same_currency_valuation_and_keeps_hanging_reads_bounded() {
    let invocations = InvocationRuntime::start(Default::default()).unwrap();
    tokio::time::pause();
    #[derive(Clone, Default)]
    struct Slow {
        calls: Arc<Mutex<Vec<ValuationRequest>>>,
        release: Arc<tokio::sync::Notify>,
    }
    impl ValuationProvider for Slow {
        async fn mark(
            &self,
            request: ValuationRequest,
        ) -> Result<Arc<market_valuation::Mark<Valuation>>, ValuationError> {
            self.calls.lock().unwrap().push(request.clone());
            self.release.notified().await;
            Quotes::default().mark(request).await
        }
    }
    let source = Source::default();
    let backend = Slow::default();
    let config = PortfolioConfig {
        valuation_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let stop = CancellationToken::new();
    let closed = CancellationToken::new();
    let handle = actor::start(
        source.clone(),
        backend.clone(),
        config,
        invocations.handle(),
        stop.clone(),
        closed.clone(),
    );
    let a = ask(&handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    let b = ask(&handle, 1, "GRAM", Priority::Interactive);
    flush().await;
    source.reply(0, Ok(page(&["g"], None, 1)));
    spin(|| backend.calls.lock().unwrap().len() == 1).await;
    backend.release.notify_one();
    let a = a.await.unwrap().unwrap();
    let b = b.await.unwrap().unwrap();
    assert_eq!(a.valuation.revision, b.valuation.revision);
    assert!(Arc::ptr_eq(&a.valuation, &b.valuation));
    assert!(Arc::ptr_eq(&a.inventory, &b.inventory));
    let stalled = ask(&handle, 1, "GRAM", Priority::Normal);
    spin(|| backend.calls.lock().unwrap().len() == 2).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        stalled.await.unwrap(),
        Err(PortfolioError::Valuation(_))
    ));
    let next = ask(&handle, 1, "GRAM", Priority::Normal);
    spin(|| backend.calls.lock().unwrap().len() == 3).await;
    backend.release.notify_one();
    next.await.unwrap().unwrap();
    assert_eq!(source.count(), 1);
    shutdown(stop, closed).await;
}

#[tokio::test]
async fn real_market_actors_supply_exact_model_floor_values() {
    let invocations = InvocationRuntime::start(Default::default()).unwrap();
    use market_hub_protocol::wire as w;
    let directory = tempfile::tempdir().unwrap();
    let mut writer =
        market_replica::ReplicaWriter::open(directory.path(), "portfolio-test").unwrap();
    let rows = vec![
        w::entity::Entity::Venue(w::Venue {
            id: "venue".into(),
            name: "Venue".into(),
        }),
        w::entity::Entity::Currency(w::Currency {
            id: "GRAM".into(),
            name: "Gram".into(),
            decimals: 9,
        }),
        w::entity::Entity::ItemType(w::ItemType {
            id: "collection".into(),
            name: "Gift".into(),
            quantity_step: "1".into(),
            aliases: vec![w::ExternalId {
                namespace: "tg".into(),
                id: "1".into(),
            }],
            attributes: [(
                "tg.model".into(),
                w::AttributeRule {
                    required: true,
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec!["model".into()],
                    }),
                },
            )]
            .into(),
        }),
        w::entity::Entity::AttributeOption(w::AttributeOption {
            id: "model".into(),
            name: "Model".into(),
            attribute_id: "tg.model".into(),
            aliases: vec![w::ExternalId {
                namespace: "tg".into(),
                id: "2".into(),
            }],
            ..Default::default()
        }),
        w::entity::Entity::Order(w::Order {
            id: "ask".into(),
            book: Some(w::BookKey {
                venue_id: "venue".into(),
                item_type_id: "collection".into(),
                currency_id: "GRAM".into(),
            }),
            mode: w::OrderMode::Ask as i32,
            price: "2.000000000000000001".into(),
            quantity: "1".into(),
            attributes: [(
                "tg.model".into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId("model".into())),
                    })),
                },
            )]
            .into(),
            ..Default::default()
        }),
    ];
    let mut seed = writer.begin_snapshot().unwrap();
    seed.append(
        &rows
            .into_iter()
            .map(|row| w::Change {
                change: Some(w::change::Change::Upsert(w::Entity { entity: Some(row) })),
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    writer
        .install_snapshot(seed.finish("portfolio-view").unwrap())
        .unwrap();
    let reads =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let market = market_actors::MarketDataService::start(reads.handle(), 2)
        .await
        .unwrap();
    let source = Source::default();
    let valuation = market_valuation::Service::start_on(
        &market.arbiter(),
        "model_floor",
        {
            let source = market_valuation::ValuationSource::new(reads.handle());
            move |life| market_valuation::FloorActor::model(source, life)
        },
        Default::default(),
    )
    .await
    .unwrap();
    let portfolio = PortfolioService::start(
        source.clone(),
        market_valuation::PortfolioValuationHandle::new([valuation.handle.clone()]).unwrap(),
        PortfolioConfig::default(),
        invocations.handle(),
    )
    .await
    .unwrap();
    let result = ask(&portfolio.handle, 1, "GRAM", Priority::Interactive);
    tokio::time::timeout(Duration::from_secs(2), async {
        while source.count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    source.reply(0, Ok(page(&["gift-a", "gift-b"], None, 2)));
    let value = result.await.unwrap().unwrap();
    assert_eq!(value.valuation.total.as_str(), "4.000000000000000002");
    assert_eq!(
        value.valuation.rows[0].floor.as_ref().unwrap().as_str(),
        "2.000000000000000001"
    );
    assert_eq!(value.valuation.revision, "portfolio-view");
    assert_eq!(value.inventory.gifts.len(), 2);
    assert_eq!(value.valuation.revision, "portfolio-view");
    assert_eq!(value.valuation.priced, 2);
    portfolio.task.shutdown().await.unwrap();
    valuation.task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    market.task.run(stop).await.unwrap();
    reads.shutdown().await;
}

#[tokio::test]
async fn dropping_lifecycle_owner_stops_work_even_with_live_handles() {
    let invocations = InvocationRuntime::start(Default::default()).unwrap();
    let source = Source::default();
    let service = PortfolioService::start(
        source.clone(),
        Quotes::default(),
        PortfolioConfig::default(),
        invocations.handle(),
    )
    .await
    .unwrap();
    let result = ask(&service.handle, 1, "GRAM", Priority::Normal);
    tokio::time::timeout(Duration::from_secs(2), async {
        while source.count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(service.task);
    assert!(matches!(
        service
            .handle
            .request(request(2, "GRAM", Priority::Normal))
            .await,
        Err(PortfolioError::Stopped)
    ));
    assert!(matches!(
        result.await.unwrap(),
        Err(PortfolioError::Stopped)
    ));
    tokio::time::timeout(Duration::from_secs(2), async {
        while !source.cancelled(0) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn unexpected_actor_failure_is_reported_to_supervision() {
    let invocations = InvocationRuntime::start(Default::default()).unwrap();
    struct Broken;
    impl InventoryProvider for Broken {
        fn page(
            &self,
            _: InventoryPageRequest,
        ) -> impl Invocation<Output = InventoryPage, Error = InventoryError> {
            operation(inventory_policy(Duration::from_secs(1)), || async {
                panic!("simulated inventory provider panic")
            })
        }
    }
    let service = PortfolioService::start(
        Broken,
        Quotes::default(),
        PortfolioConfig::default(),
        invocations.handle(),
    )
    .await
    .unwrap();
    let pending = ask(&service.handle, 1, "GRAM", Priority::Normal);
    assert!(matches!(
        pending.await.unwrap(),
        Err(PortfolioError::Stopped)
    ));
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        service.task.run(CancellationToken::new()),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(PortfolioError::Stopped)));
}

#[actix::test]
async fn refreshed_inventory_does_not_join_valuation_of_an_expired_snapshot() {
    let invocations = InvocationRuntime::start(Default::default()).unwrap();
    tokio::time::pause();
    #[derive(Clone, Default)]
    struct Slow {
        calls: Arc<Mutex<Vec<ValuationRequest>>>,
        release: Arc<tokio::sync::Notify>,
    }
    impl ValuationProvider for Slow {
        async fn mark(
            &self,
            request: ValuationRequest,
        ) -> Result<Arc<market_valuation::Mark<Valuation>>, ValuationError> {
            self.calls.lock().unwrap().push(request.clone());
            self.release.notified().await;
            Quotes::default().mark(request).await
        }
    }
    let source = Source::default();
    let backend = Slow::default();
    let stop = CancellationToken::new();
    let closed = CancellationToken::new();
    let config = PortfolioConfig {
        inventory_ttl: Duration::from_secs(1),
        ..Default::default()
    };
    let handle = actor::start(
        source.clone(),
        backend.clone(),
        config,
        invocations.handle(),
        stop.clone(),
        closed.clone(),
    );
    let old = ask(&handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    source.reply(0, Ok(page(&["a"], None, 1)));
    spin(|| backend.calls.lock().unwrap().len() == 1).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    let fresh = ask(&handle, 1, "GRAM", Priority::Interactive);
    next(&source, 2).await;
    source.reply(1, Ok(page(&["a", "b"], None, 2)));
    flush().await;
    assert_eq!(
        backend.calls.lock().unwrap().len(),
        1,
        "valuations remain serialized"
    );
    backend.release.notify_one();
    let old = old.await.unwrap().unwrap();
    spin(|| backend.calls.lock().unwrap().len() == 2).await;
    backend.release.notify_one();
    let fresh = fresh.await.unwrap().unwrap();
    assert_eq!(old.valuation.total.as_str(), "2");
    assert_eq!(fresh.valuation.total.as_str(), "4");
    assert!(!Arc::ptr_eq(&old.inventory, &fresh.inventory));
    assert!(!Arc::ptr_eq(&old.valuation, &fresh.valuation));
    shutdown(stop, closed).await;
}

#[test]
fn queue_assigns_four_two_one_turns_under_continuous_load() {
    let mut queue = queue::Queue::default();
    for n in 0..12 {
        queue.insert(n, Priority::Interactive);
        queue.insert(100 + n, Priority::Normal);
        queue.insert(200 + n, Priority::Background);
    }
    let round: Vec<_> = (0..7).map(|_| queue.pop_ready(|_| true).unwrap()).collect();
    assert_eq!(round, vec![0, 1, 2, 3, 100, 101, 200]);
    let round: Vec<_> = (0..7).map(|_| queue.pop_ready(|_| true).unwrap()).collect();
    assert_eq!(round, vec![4, 5, 6, 7, 102, 103, 201]);
}

#[tokio::test]
async fn shared_invocation_shutdown_stops_portfolio_supervision_and_cancels_active_pages() {
    let invocations = InvocationRuntime::start(Default::default()).unwrap();
    let source = Source::default();
    let service = PortfolioService::start(
        source.clone(),
        Quotes::default(),
        PortfolioConfig::default(),
        invocations.handle(),
    )
    .await
    .unwrap();
    let result = ask(&service.handle, 1, "GRAM", Priority::Normal);
    next(&source, 1).await;
    invocations.shutdown().await;
    assert!(matches!(
        service.task.run(CancellationToken::new()).await,
        Err(PortfolioError::Stopped)
    ));
    assert!(matches!(
        result.await.unwrap(),
        Err(PortfolioError::Stopped)
    ));
    assert!(source.cancelled(0));
}

#[actix::test]
async fn strategies_share_inventory_but_never_coalesce_valuation_jobs() {
    tokio::time::pause();
    let source = Source::default();
    let quotes = Quotes::default();
    let (handle, stop, closed, _invocations) =
        setup(source.clone(), quotes.clone(), Default::default());
    let model = ask(&handle, 1, "GRAM", Priority::Normal);
    let second = handle.clone();
    let mut broad = request(1, "GRAM", Priority::Interactive);
    broad.strategy = market_valuation::StrategyId::collection_floor();
    let collection = tokio::spawn(async move { second.request(broad).await });
    next(&source, 1).await;
    flush().await;
    source.reply(0, Ok(page(&["one", "two"], None, 2)));
    let a = model.await.unwrap().unwrap();
    let b = collection.await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&a.inventory, &b.inventory));
    assert_eq!(source.count(), 1);
    let strategies: std::collections::BTreeSet<_> = quotes
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.strategy.clone())
        .collect();
    assert_eq!(strategies.len(), 2);
    shutdown(stop, closed).await;
}

#[actix::test]
async fn gift_strategy_is_rejected_before_inventory_is_requested() {
    let source = Source::default();
    let quotes = Quotes::default();
    let (handle, stop, closed, _invocations) =
        setup(source.clone(), quotes.clone(), Default::default());
    let mut request = request(1, "GRAM", Priority::Normal);
    request.strategy = market_valuation::StrategyId::new("gift_floor").unwrap();
    let error = handle.request(request).await.unwrap_err();
    let PortfolioError::Valuation(error) = error else {
        panic!("strategy error required");
    };
    assert_eq!(
        error.code(),
        market_valuation::ValuationErrorCode::UnsupportedStrategy
    );
    assert_eq!(source.count(), 0);
    assert!(quotes.calls.lock().unwrap().is_empty());
    shutdown(stop, closed).await;
}
