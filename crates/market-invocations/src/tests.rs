use super::*;
use std::{
    collections::VecDeque,
    sync::{Mutex, atomic::AtomicUsize},
};
use tokio::task::JoinHandle;
fn key(group: &'static str) -> RateKey {
    RateKey {
        owner: 1,
        group,
        resource: None,
    }
}
fn policy(group: &'static str) -> Policy {
    Policy {
        limits: vec![RateLimit {
            key: key(group),
            interval: Duration::from_millis(100),
            concurrency: 1,
        }],
        replay: Replay::Safe,
        attempt_timeout: Duration::from_secs(10),
        max_attempts: 3,
    }
}
#[derive(Clone)]
struct Script {
    id: u32,
    policy: Policy,
    log: Arc<Mutex<Vec<u32>>>,
    results: Arc<Mutex<VecDeque<Result<u32, Retry>>>>,
}
impl Script {
    fn new(id: u32, group: &'static str, log: &Arc<Mutex<Vec<u32>>>) -> Self {
        Self {
            id,
            policy: policy(group),
            log: log.clone(),
            results: Default::default(),
        }
    }
}
impl Invocation for Script {
    type Output = u32;
    type Error = &'static str;
    fn policy(&self) -> Policy {
        self.policy.clone()
    }
    async fn execute(&self) -> Result<u32, AttemptError<&'static str>> {
        self.log.lock().unwrap().push(self.id);
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(self.id))
            .map_err(|retry| AttemptError {
                error: "remote",
                retry,
            })
    }
}
async fn flush() {
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
}
fn start(
    handle: &Handle,
    op: Script,
    priority: Priority,
) -> JoinHandle<Result<u32, Error<&'static str>>> {
    tokio::spawn(
        handle
            .submit(
                op,
                Options {
                    priority,
                    ..Default::default()
                },
            )
            .unwrap(),
    )
}
#[tokio::test(start_paused = true)]
async fn shares_rate_slots_and_admits_unrelated_groups_without_head_of_line_blocking() {
    let rt = Runtime::start(Default::default()).unwrap();
    let h = rt.handle();
    let log = Default::default();
    start(&h, Script::new(1, "gifts", &log), Priority::Normal)
        .await
        .unwrap()
        .unwrap();
    let blocked = start(&h, Script::new(2, "gifts", &log), Priority::Interactive);
    let other = start(&h, Script::new(3, "media", &log), Priority::Background);
    flush().await;
    assert_eq!(*log.lock().unwrap(), [1, 3]);
    other.await.unwrap().unwrap();
    tokio::time::advance(Duration::from_millis(100)).await;
    blocked.await.unwrap().unwrap();
    assert_eq!(*log.lock().unwrap(), [1, 3, 2]);
    rt.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn weighted_priorities_are_fifo_and_background_does_not_starve() {
    let rt = Runtime::start(Default::default()).unwrap();
    let h = rt.handle();
    let log = Default::default();
    start(&h, Script::new(0, "gifts", &log), Priority::Normal)
        .await
        .unwrap()
        .unwrap();
    // Reset the normal credit consumed by the initial pacing request in expected order below.
    let mut jobs = vec![];
    for (base, p) in [
        (10, Priority::Interactive),
        (20, Priority::Normal),
        (30, Priority::Background),
    ] {
        for id in base..base + 6 {
            jobs.push(start(&h, Script::new(id, "gifts", &log), p));
        }
    }
    flush().await;
    for _ in 0..18 {
        tokio::time::advance(Duration::from_millis(100)).await;
        flush().await;
    }
    for j in jobs {
        j.await.unwrap().unwrap();
    }
    let entries = log.lock().unwrap().clone();
    assert_eq!(&entries[..7], &[0, 10, 11, 12, 13, 20, 30]);
    for base in [10, 20, 30] {
        let class: Vec<_> = entries
            .iter()
            .copied()
            .filter(|n| *n >= base && *n < base + 6)
            .collect();
        assert_eq!(class, (base..base + 6).collect::<Vec<_>>());
    }
    rt.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn queued_work_can_be_promoted_and_demoted() {
    let rt = Runtime::start(Default::default()).unwrap();
    let h = rt.handle();
    let log = Default::default();
    start(&h, Script::new(0, "gifts", &log), Priority::Normal)
        .await
        .unwrap()
        .unwrap();
    let ticket = h
        .submit(
            Script::new(1, "gifts", &log),
            Options {
                priority: Priority::Background,
                ..Default::default()
            },
        )
        .unwrap();
    let control = ticket.control();
    let promoted = tokio::spawn(ticket);
    let other = start(&h, Script::new(2, "gifts", &log), Priority::Normal);
    flush().await;
    control.set_priority(Priority::Interactive);
    tokio::time::advance(Duration::from_millis(100)).await;
    flush().await;
    assert_eq!(*log.lock().unwrap(), [0, 1]);
    tokio::time::advance(Duration::from_millis(100)).await;
    promoted.await.unwrap().unwrap();
    other.await.unwrap().unwrap();
    rt.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn full_flood_wait_survives_cancellation_and_connection_generations() {
    let rt = Runtime::start(Default::default()).unwrap();
    let h = rt.handle();
    let log = Default::default();
    let op = Script::new(1, "gifts", &log);
    op.results.lock().unwrap().push_back(Err(Retry::After {
        delay: Duration::from_secs(7200),
        scope: key("gifts"),
    }));
    let generation = CancellationToken::new();
    let job = tokio::spawn(
        h.submit(
            op,
            Options {
                cancellation: generation.clone(),
                timeout: Duration::from_secs(10000),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    flush().await;
    generation.cancel();
    assert_eq!(
        job.await.unwrap(),
        Err(Error::Runtime(RuntimeError::Cancelled))
    );
    let other = start(&h, Script::new(2, "media", &log), Priority::Normal);
    other.await.unwrap().unwrap();
    let job = tokio::spawn(
        h.submit(
            Script::new(3, "gifts", &log),
            Options {
                timeout: Duration::from_secs(10000),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    flush().await;
    tokio::time::advance(Duration::from_secs(7199)).await;
    flush().await;
    assert_eq!(*log.lock().unwrap(), [1, 2]);
    tokio::time::advance(Duration::from_secs(1)).await;
    job.await.unwrap().unwrap();
    assert_eq!(*log.lock().unwrap(), [1, 2, 3]);
    rt.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn multiple_scopes_are_admitted_atomically_and_never_shorten_a_cooldown() {
    let rt = Runtime::start(Default::default()).unwrap();
    let h = rt.handle();
    let log = Default::default();
    let mut slow = Script::new(1, "account", &log);
    slow.results.lock().unwrap().push_back(Err(Retry::After {
        delay: Duration::from_secs(5),
        scope: key("account"),
    }));
    slow.policy.replay = Replay::Never;
    start(&h, slow, Priority::Normal)
        .await
        .unwrap()
        .unwrap_err();
    let mut op = Script::new(2, "gifts", &log);
    op.policy.limits.push(policy("account").limits[0].clone());
    let job = start(&h, op, Priority::Normal);
    flush().await;
    // The blocked account must not reserve the independent gifts slot.
    start(&h, Script::new(3, "gifts", &log), Priority::Background)
        .await
        .unwrap()
        .unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;
    job.await.unwrap().unwrap();
    assert_eq!(*log.lock().unwrap(), [1, 3, 2]);
    rt.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn deadline_returns_remaining_rate_wait_without_erasing_it() {
    let rt = Runtime::start(Default::default()).unwrap();
    let h = rt.handle();
    let log = Default::default();
    let mut op = Script::new(1, "gifts", &log);
    op.policy.replay = Replay::Never;
    op.results.lock().unwrap().push_back(Err(Retry::After {
        delay: Duration::from_secs(20),
        scope: key("gifts"),
    }));
    start(&h, op, Priority::Normal).await.unwrap().unwrap_err();
    let ticket = h
        .submit(
            Script::new(2, "gifts", &log),
            Options {
                timeout: Duration::from_secs(3),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        ticket.await,
        Err(Error::Runtime(RuntimeError::Deadline {
            retry_after: Some(Duration::from_secs(17))
        }))
    );
    assert_eq!(*log.lock().unwrap(), [1]);
    rt.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn admission_is_bounded_even_before_polling_and_cancellation_releases_capacity() {
    let rt = Runtime::start(Config {
        max_pending: 1,
        max_executing: 1,
        ..Default::default()
    })
    .unwrap();
    let h = rt.handle();
    let log = Default::default();
    let ticket = h
        .submit(Script::new(1, "gifts", &log), Default::default())
        .unwrap();
    assert!(matches!(
        h.submit(Script::new(2, "gifts", &log), Default::default()),
        Err(RuntimeError::Busy)
    ));
    drop(ticket);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let ticket = h
        .submit(
            Script::new(3, "gifts", &log),
            Options {
                cancellation: cancel,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(ticket.await, Err(Error::Runtime(RuntimeError::Cancelled)));
    assert!(log.lock().unwrap().is_empty());
    rt.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn dropping_queued_tickets_does_not_accumulate_or_execute_abandoned_work() {
    let rt = Runtime::start(Config {
        max_pending: 2,
        max_executing: 1,
        max_scopes: 1,
    })
    .unwrap();
    let h = rt.handle();
    let log = Default::default();
    start(&h, Script::new(0, "gifts", &log), Priority::Normal)
        .await
        .unwrap()
        .unwrap();
    for n in 1..100 {
        let j = start(&h, Script::new(n, "gifts", &log), Priority::Normal);
        flush().await;
        j.abort();
        let _ = j.await;
        flush().await;
    }
    tokio::time::advance(Duration::from_millis(100)).await;
    flush().await;
    assert_eq!(*log.lock().unwrap(), [0]);
    rt.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn shutdown_and_attempt_timeout_drop_running_execution_and_keep_scheduler_healthy() {
    let rt = Runtime::start(Default::default()).unwrap();
    let h = rt.handle();
    let dropped = Arc::new(AtomicUsize::new(0));
    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let op = operation(policy("gifts"), {
        let dropped = dropped.clone();
        move || {
            let dropped = dropped.clone();
            async move {
                let _g = Guard(dropped);
                std::future::pending::<Result<(), AttemptError<()>>>().await
            }
        }
    });
    let ticket = h.submit(op, Default::default()).unwrap();
    let job = tokio::spawn(ticket);
    flush().await;
    rt.shutdown().await;
    assert_eq!(
        job.await.unwrap(),
        Err(Error::Runtime(RuntimeError::Stopped))
    );
    assert_eq!(dropped.load(Ordering::Relaxed), 1);
    assert!(matches!(
        h.submit(
            Script::new(1, "gifts", &Default::default()),
            Default::default()
        ),
        Err(RuntimeError::Stopped)
    ));
    let rt = Runtime::start(Default::default()).unwrap();
    let h = rt.handle();
    let op = operation(policy("gifts"), || async {
        std::future::pending::<Result<(), AttemptError<()>>>().await
    });
    assert_eq!(
        h.submit(op, Default::default()).unwrap().await,
        Err(Error::Runtime(RuntimeError::AttemptTimeout))
    );
    start(
        &h,
        Script::new(1, "gifts", &Default::default()),
        Priority::Normal,
    )
    .await
    .unwrap()
    .unwrap();
    rt.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn retries_only_replayable_operations_with_bounded_attempts() {
    let rt = Runtime::start(Default::default()).unwrap();
    let h = rt.handle();
    let log = Default::default();
    for replay in [Replay::Safe, Replay::Deduplicated, Replay::Never] {
        let mut op = Script::new(1, "gifts", &log);
        op.policy.replay = replay;
        op.results.lock().unwrap().extend([
            Err(Retry::Transient),
            Err(Retry::Transient),
            Err(Retry::Transient),
        ]);
        assert_eq!(
            h.submit(op, Default::default()).unwrap().await,
            Err(Error::Operation("remote"))
        );
    }
    assert_eq!(log.lock().unwrap().len(), 7);
    rt.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn scope_capacity_does_not_evict_live_cooldowns_and_invalid_policies_fail() {
    let rt = Runtime::start(Config {
        max_scopes: 1,
        ..Default::default()
    })
    .unwrap();
    let h = rt.handle();
    let log = Default::default();
    let mut op = Script::new(1, "gifts", &log);
    op.policy.replay = Replay::Never;
    op.results.lock().unwrap().push_back(Err(Retry::After {
        delay: Duration::from_secs(100),
        scope: key("gifts"),
    }));
    start(&h, op, Priority::Normal).await.unwrap().unwrap_err();
    assert_eq!(
        start(&h, Script::new(2, "media", &log), Priority::Normal)
            .await
            .unwrap(),
        Err(Error::Runtime(RuntimeError::Busy))
    );
    let mut op = Script::new(3, "gifts", &log);
    op.policy.limits.push(op.policy.limits[0].clone());
    assert!(matches!(
        h.submit(op, Default::default()),
        Err(RuntimeError::InvalidPolicy)
    ));
    rt.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn expired_unpolled_tickets_never_execute_and_mapping_preserves_retry_policy() {
    let runtime = Runtime::start(Default::default()).unwrap();
    let handle = runtime.handle();
    let log = Default::default();
    let ticket = handle
        .submit(
            Script::new(1, "gifts", &log),
            Options {
                timeout: Duration::from_secs(1),
                ..Default::default()
            },
        )
        .unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(
        ticket.await,
        Err(Error::Runtime(RuntimeError::Deadline { retry_after: None }))
    );
    assert!(log.lock().unwrap().is_empty());
    let source = Script::new(2, "gifts", &log);
    source.results.lock().unwrap().push_back(Err(Retry::After {
        delay: Duration::from_secs(100),
        scope: key("gifts"),
    }));
    let mapped = source.map(|n| n.to_string(), str::to_owned);
    let error = handle
        .submit(
            mapped,
            Options {
                timeout: Duration::from_secs(1),
                ..Default::default()
            },
        )
        .unwrap()
        .await;
    assert!(
        matches!(error,Err(Error::Runtime(RuntimeError::Deadline{retry_after:Some(wait)})) if wait==Duration::from_secs(99))
    );
    assert_eq!(*log.lock().unwrap(), [2]);
    runtime.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn global_execution_bound_is_shared_across_unrelated_scopes() {
    let runtime = Runtime::start(Config {
        max_executing: 2,
        ..Default::default()
    })
    .unwrap();
    let handle = runtime.handle();
    let active = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Semaphore::new(0));
    let mut tasks = vec![];
    for group in ["one", "two", "three"] {
        let active = active.clone();
        let gate = gate.clone();
        tasks.push(tokio::spawn(
            handle
                .submit(
                    operation(policy(group), move || {
                        let active = active.clone();
                        let gate = gate.clone();
                        async move {
                            active.fetch_add(1, Ordering::Relaxed);
                            gate.acquire().await.unwrap().forget();
                            Ok::<_, AttemptError<()>>(())
                        }
                    }),
                    Options::default(),
                )
                .unwrap(),
        ));
    }
    flush().await;
    assert_eq!(active.load(Ordering::Relaxed), 2);
    gate.add_permits(1);
    flush().await;
    assert_eq!(active.load(Ordering::Relaxed), 3);
    gate.add_permits(2);
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    runtime.shutdown().await;
}
#[tokio::test(start_paused = true)]
async fn shorter_overlapping_flood_waits_cannot_release_a_longer_cooldown() {
    let runtime = Runtime::start(Default::default()).unwrap();
    let handle = runtime.handle();
    let release = Arc::new(tokio::sync::Notify::new());
    let ready = Arc::new(AtomicUsize::new(0));
    let mut rate = policy("account");
    rate.limits[0].interval = Duration::ZERO;
    rate.limits[0].concurrency = 2;
    rate.replay = Replay::Never;
    let wait = release.clone();
    let entered = ready.clone();
    let second = tokio::spawn(
        handle
            .submit(
                operation(rate.clone(), move || {
                    let wait = wait.clone();
                    let entered = entered.clone();
                    async move {
                        entered.fetch_add(1, Ordering::Relaxed);
                        wait.notified().await;
                        Err::<(), _>(AttemptError {
                            error: (),
                            retry: Retry::After {
                                delay: Duration::from_secs(10),
                                scope: key("account"),
                            },
                        })
                    }
                }),
                Options {
                    timeout: Duration::from_secs(200),
                    ..Default::default()
                },
            )
            .unwrap(),
    );
    flush().await;
    assert_eq!(ready.load(Ordering::Relaxed), 1);
    handle
        .submit(
            operation(rate.clone(), || async {
                Err::<(), _>(AttemptError {
                    error: (),
                    retry: Retry::After {
                        delay: Duration::from_secs(100),
                        scope: key("account"),
                    },
                })
            }),
            Options::default(),
        )
        .unwrap()
        .await
        .unwrap_err();
    tokio::time::advance(Duration::from_secs(10)).await;
    release.notify_one();
    second.await.unwrap().unwrap_err();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let third = tokio::spawn(
        handle
            .submit(
                operation(rate, move || {
                    seen.fetch_add(1, Ordering::Relaxed);
                    async { Ok::<_, AttemptError<()>>(()) }
                }),
                Options {
                    timeout: Duration::from_secs(200),
                    ..Default::default()
                },
            )
            .unwrap(),
    );
    tokio::time::advance(Duration::from_secs(11)).await;
    flush().await;
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    tokio::time::advance(Duration::from_secs(79)).await;
    third.await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    runtime.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn aborted_scheduler_closes_handles_even_if_it_never_started() {
    let runtime = Runtime::start(Default::default()).unwrap();
    let handle = runtime.handle();
    runtime.task.as_ref().unwrap().abort();
    tokio::time::timeout(Duration::from_secs(1), handle.closed())
        .await
        .unwrap();
    assert!(handle.is_closed());
    assert!(matches!(
        handle.submit(
            Script::new(1, "gifts", &Default::default()),
            Options::default()
        ),
        Err(RuntimeError::Stopped)
    ));
    runtime.shutdown().await;
}
