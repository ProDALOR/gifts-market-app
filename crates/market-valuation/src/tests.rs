use super::*;
use std::sync::Mutex;
struct Call {
    key: u8,
    reply: oneshot::Sender<Result<Evaluation<u32>, &'static str>>,
}
#[derive(Clone)]
struct Source {
    calls: Arc<Mutex<Vec<Call>>>,
    changed: watch::Sender<u64>,
}
impl Source {
    fn new() -> Self {
        Self {
            calls: Default::default(),
            changed: watch::channel(0).0,
        }
    }
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    fn answer(&self, n: usize, value: u32) {
        let call = self.calls.lock().unwrap().remove(n);
        let _ = call.reply.send(Ok(Evaluation {
            value,
            revision: value.to_string(),
            coverage: Coverage {
                priced: 1,
                unpriced: 0,
            },
        }));
    }
}
impl Strategy for Source {
    type Key = u8;
    type Value = u32;
    type Error = &'static str;
    fn normalize(&self, key: u8) -> Result<u8, Self::Error> {
        if key == 255 {
            Err("invalid key")
        } else {
            Ok(key % 10)
        }
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
    async fn evaluate(&self, key: u8) -> Result<Evaluation<u32>, Self::Error> {
        let (reply, rx) = oneshot::channel();
        self.calls.lock().unwrap().push(Call { key, reply });
        rx.await.unwrap_or(Err("cancelled"))
    }
}
async fn flush() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}
fn ask(
    handle: &Handle<u8, u32, &'static str>,
    key: u8,
) -> tokio::task::JoinHandle<Result<Arc<Mark<u32>>, Error<&'static str>>> {
    let handle = handle.clone();
    tokio::spawn(async move { handle.mark(key).await })
}
#[tokio::test(start_paused = true)]
async fn normalizes_coalesces_and_serves_the_same_cached_mark() {
    let source = Source::new();
    let service = Service::start(source.clone(), Default::default()).unwrap();
    let a = ask(&service.handle, 1);
    let b = ask(&service.handle, 11);
    flush().await;
    assert_eq!(source.count(), 1);
    assert_eq!(source.calls.lock().unwrap()[0].key, 1);
    source.answer(0, 42);
    let a = a.await.unwrap().unwrap();
    let b = b.await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    let c = service.handle.mark(1).await.unwrap();
    assert!(Arc::ptr_eq(&a, &c));
    assert_eq!(a.revision, "42");
    assert_eq!(a.coverage.priced, 1);
    assert_eq!(
        service.handle.mark(255).await,
        Err(Error::Strategy("invalid key"))
    );
    service.task.shutdown().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn invalidation_discards_an_older_inflight_evaluation() {
    let source = Source::new();
    let service = Service::start(source.clone(), Default::default()).unwrap();
    let result = ask(&service.handle, 1);
    flush().await;
    source.changed.send_replace(1);
    source.answer(0, 1);
    flush().await;
    assert!(!result.is_finished());
    assert_eq!(source.count(), 1);
    source.answer(0, 2);
    let mark = result.await.unwrap().unwrap();
    assert_eq!(mark.value, 2);
    service.task.shutdown().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn refreshes_on_changes_and_periodically_without_keepalive_of_idle_entries() {
    let source = Source::new();
    let config = Config {
        idle_ttl: std::time::Duration::from_secs(60),
        ..Default::default()
    };
    let service = Service::start(source.clone(), config).unwrap();
    let first = ask(&service.handle, 1);
    flush().await;
    source.answer(0, 1);
    first.await.unwrap().unwrap();
    source.changed.send_replace(1);
    flush().await;
    assert_eq!(source.count(), 0);
    tokio::time::advance(Duration::from_millis(101)).await;
    flush().await;
    assert_eq!(source.count(), 1);
    source.answer(0, 2);
    flush().await;
    assert_eq!(service.handle.mark(1).await.unwrap().value, 2);
    tokio::time::advance(Duration::from_secs(30)).await;
    flush().await;
    assert_eq!(source.count(), 1);
    source.answer(0, 3);
    flush().await;
    assert_eq!(service.handle.mark(1).await.unwrap().value, 3);
    tokio::time::advance(Duration::from_secs(61)).await;
    flush().await;
    assert_eq!(source.count(), 0);
    service.task.shutdown().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn failures_and_timeouts_are_not_cached_or_substituted_with_stale_values() {
    let source = Source::new();
    let service = Service::start(source.clone(), Default::default()).unwrap();
    let a = ask(&service.handle, 1);
    let b = ask(&service.handle, 1);
    flush().await;
    source
        .calls
        .lock()
        .unwrap()
        .remove(0)
        .reply
        .send(Err("failed"))
        .unwrap();
    assert_eq!(a.await.unwrap(), Err(Error::Strategy("failed")));
    assert_eq!(b.await.unwrap(), Err(Error::Strategy("failed")));
    let retry = ask(&service.handle, 1);
    flush().await;
    assert_eq!(source.count(), 1);
    tokio::time::advance(Duration::from_secs(10)).await;
    assert_eq!(retry.await.unwrap(), Err(Error::TimedOut));
    service.task.shutdown().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn admission_cache_and_shutdown_are_bounded() {
    let source = Source::new();
    let service = Service::start(
        source.clone(),
        Config {
            max_requests: 2,
            max_entries: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let first = ask(&service.handle, 1);
    flush().await;
    let second = ask(&service.handle, 2);
    flush().await;
    assert_eq!(second.await.unwrap(), Err(Error::Busy));
    let duplicate = ask(&service.handle, 1);
    flush().await;
    assert_eq!(service.handle.mark(3).await, Err(Error::Busy));
    first.abort();
    let _ = first.await;
    service.task.shutdown().await.unwrap();
    assert_eq!(duplicate.await.unwrap(), Err(Error::Stopped));
    assert!(source.calls.lock().unwrap()[0].reply.is_closed());
    assert_eq!(service.handle.mark(1).await, Err(Error::Stopped));
}

#[tokio::test(start_paused = true)]
async fn abandoning_queued_callers_releases_admission_and_does_not_evaluate_their_keys() {
    let source = Source::new();
    let service = Service::start(
        source.clone(),
        Config {
            max_requests: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let first = ask(&service.handle, 1);
    flush().await;
    let abandoned = ask(&service.handle, 2);
    flush().await;
    abandoned.abort();
    let _ = abandoned.await;
    flush().await;
    source.answer(0, 1);
    first.await.unwrap().unwrap();
    flush().await;
    assert_eq!(source.count(), 0);
    let next = ask(&service.handle, 3);
    flush().await;
    assert_eq!(source.calls.lock().unwrap()[0].key, 3);
    source.answer(0, 3);
    next.await.unwrap().unwrap();
    service.task.shutdown().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn dropped_lifecycle_owner_closes_handles_and_aborts_active_work() {
    let source = Source::new();
    let service = Service::start(source.clone(), Default::default()).unwrap();
    let result = ask(&service.handle, 1);
    flush().await;
    drop(service.task);
    assert_eq!(result.await.unwrap(), Err(Error::Stopped));
    service.handle.closed().await;
    assert!(source.calls.lock().unwrap()[0].reply.is_closed());
    assert_eq!(service.handle.mark(2).await, Err(Error::Stopped));
}

#[tokio::test(start_paused = true)]
async fn strategy_panics_are_reported_and_never_leave_waiters_hanging() {
    struct Broken(tokio::sync::watch::Receiver<u64>);
    impl Strategy for Broken {
        type Key = u8;
        type Value = u32;
        type Error = &'static str;
        fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
            self.0.clone()
        }
        async fn evaluate(&self, _: u8) -> Result<Evaluation<u32>, Self::Error> {
            panic!("simulated valuation failure")
        }
    }
    let (_sender, changes) = tokio::sync::watch::channel(0);
    let service = Service::start(Broken(changes), Default::default()).unwrap();
    let result = ask(&service.handle, 1);
    assert_eq!(result.await.unwrap(), Err(Error::Stopped));
    assert!(service.task.run(CancellationToken::new()).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn bounded_mark_cache_evicts_inactive_keys_and_recomputes_on_next_read() {
    let source = Source::new();
    let service = Service::start(
        source.clone(),
        Config {
            max_entries: 1,
            ..Default::default()
        },
    )
    .unwrap();
    for key in [1, 2, 1] {
        let result = ask(&service.handle, key);
        flush().await;
        assert_eq!(source.count(), 1);
        assert_eq!(source.calls.lock().unwrap()[0].key, key);
        source.answer(0, u32::from(key));
        assert_eq!(result.await.unwrap().unwrap().value, u32::from(key));
    }
    service.task.shutdown().await.unwrap();
}
