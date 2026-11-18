use super::*;
use actix::{AtomicResponse, WrapFuture};
use std::sync::atomic::{AtomicUsize, Ordering};

type TestMessage = Evaluate<u8, u64, &'static str>;
struct TestActor {
    life: Lifecycle,
    gate: Arc<Semaphore>,
    entered: Arc<AtomicUsize>,
}
impl Actor for TestActor {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.life.started(ctx);
    }
}
impl Handler<TestMessage> for TestActor {
    type Result = AtomicResponse<Self, Result<Arc<Mark<u64>>, &'static str>>;
    fn handle(&mut self, message: TestMessage, _: &mut Self::Context) -> Self::Result {
        let (input, request) = message.into_parts();
        self.entered.fetch_add(1, Ordering::SeqCst);
        assert_ne!(input, 254, "synthetic actor panic");
        let stop = self.life.stop_token();
        let gate = self.gate.clone();
        AtomicResponse::new(Box::pin(
            async move {
                let _request = request;
                if input == 255 {
                    return Err("invalid input");
                }
                tokio::select! { biased;
                    _ = stop.cancelled() => return Err("stopping"),
                    _ = _request.cancelled.cancelled() => return Err("cancelled"),
                    permit = gate.acquire() => permit.unwrap().forget(),
                }
                Ok(u64::from(input) * 2)
            }
            .into_actor(self)
            .map(|value, actor, _| {
                value.map(|data| {
                    Arc::new(Mark::evaluated(
                        actor.life.strategy(),
                        Prepared {
                            prepared_at: jiff::Timestamp::now(),
                            data,
                            revision: "pinned-revision".into(),
                        },
                    ))
                })
            }),
        ))
    }
}
fn start(
    config: Config,
) -> (
    Service<u8, u64, &'static str>,
    Arc<Semaphore>,
    Arc<AtomicUsize>,
) {
    let gate = Arc::new(Semaphore::new(0));
    let entered = Arc::new(AtomicUsize::new(0));
    let service = Service::start("test", config, |life| TestActor {
        life,
        gate: gate.clone(),
        entered: entered.clone(),
    })
    .unwrap();
    (service, gate, entered)
}
async fn until_entered(entered: &AtomicUsize, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while entered.load(Ordering::SeqCst) < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[actix::test]
async fn typed_results_and_strategy_errors_keep_identity_and_revision() {
    let (service, gate, _) = start(Config::default());
    gate.add_permits(1);
    let mark = service.handle.evaluate(4).await.unwrap();
    assert_eq!(mark.data, 8);
    assert_eq!(mark.strategy, "test");
    assert_eq!(mark.revision, "pinned-revision");
    assert_eq!(
        service.handle.evaluate(255).await,
        Err(Error::Strategy("invalid input"))
    );
    service.task.shutdown().await.unwrap();
}
#[actix::test]
async fn native_actor_admission_is_bounded_and_caller_cancellation_unblocks_next_request() {
    let (service, gate, entered) = start(Config {
        max_requests: 1,
        ..Default::default()
    });
    let handle = service.handle.clone();
    let first = tokio::spawn(async move { handle.evaluate(1).await });
    until_entered(&entered, 1).await;
    assert_eq!(service.handle.evaluate(2).await, Err(Error::Busy));
    first.abort();
    let _ = first.await;
    gate.add_permits(1);
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match service.handle.evaluate(3).await {
                Err(Error::Busy) => tokio::task::yield_now().await,
                other => break other,
            }
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.data, 6);
    service.task.shutdown().await.unwrap();
}
#[actix::test]
async fn timeout_cancels_native_work_and_releases_admission() {
    let (service, gate, entered) = start(Config {
        max_requests: 1,
        request_timeout: Duration::from_millis(30),
    });
    assert_eq!(service.handle.evaluate(1).await, Err(Error::TimedOut));
    until_entered(&entered, 1).await;
    gate.add_permits(1);
    let mark = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match service.handle.evaluate(2).await {
                Err(Error::Busy) => tokio::task::yield_now().await,
                other => break other,
            }
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(mark.data, 4);
    service.task.shutdown().await.unwrap();
}
#[actix::test]
async fn stopping_actor_interrupts_pending_response_and_closes_handle() {
    let (service, _, entered) = start(Config::default());
    let handle = service.handle.clone();
    let request_handle = handle.clone();
    let request = tokio::spawn(async move { request_handle.evaluate(1).await });
    until_entered(&entered, 1).await;
    tokio::time::timeout(Duration::from_secs(2), service.task.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.await.unwrap(), Err(Error::Stopped));
    handle.closed().await;
    assert_eq!(handle.evaluate(2).await, Err(Error::Stopped));
}
#[actix::test]
async fn supervision_reports_unexpected_actor_exit() {
    let (service, _, _) = start(Config::default());
    assert_eq!(service.handle.evaluate(254).await, Err(Error::Stopped));
    assert_eq!(
        service.task.run(CancellationToken::new()).await,
        Err(ServiceError)
    );
}
#[actix::test]
async fn supervisor_cancellation_and_task_drop_stop_native_actor() {
    let (service, _, _) = start(Config::default());
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
    assert_eq!(service.handle.evaluate(1).await, Err(Error::Stopped));
    let (service, _, _) = start(Config::default());
    drop(service.task);
    tokio::time::timeout(Duration::from_secs(2), service.handle.closed())
        .await
        .unwrap();
}
#[actix::test]
async fn rejects_invalid_configuration_before_building_actor() {
    for (id, config) in [
        ("", Config::default()),
        (
            "test",
            Config {
                max_requests: 0,
                ..Default::default()
            },
        ),
        (
            "test",
            Config {
                max_requests: 4097,
                ..Default::default()
            },
        ),
        (
            "test",
            Config {
                request_timeout: Duration::ZERO,
                ..Default::default()
            },
        ),
    ] {
        let result = Service::<u8, u64, &'static str>::start::<TestActor, _>(id, config, |_| {
            panic!("invalid factory invoked")
        });
        assert!(result.is_err());
    }
}
#[tokio::test]
async fn requires_actix_context() {
    assert!(
        Service::<u8, u64, &'static str>::start::<TestActor, _>(
            "test",
            Config::default(),
            |_| panic!("factory invoked without Actix")
        )
        .is_err()
    );
}

#[actix::test]
async fn cancelled_queued_request_never_executes_or_retains_capacity() {
    let (service, gate, entered) = start(Config {
        max_requests: 2,
        ..Default::default()
    });
    let first = tokio::spawn(service.handle.evaluate(1));
    until_entered(&entered, 1).await;
    let mut queued = Box::pin(service.handle.evaluate(2));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut queued)
            .await
            .is_err()
    );
    assert_eq!(service.handle.evaluate(3).await, Err(Error::Busy));
    drop(queued);
    gate.add_permits(2);
    first.await.unwrap().unwrap();
    assert_eq!(service.handle.evaluate(3).await.unwrap().data, 6);
    assert_eq!(entered.load(Ordering::SeqCst), 2);
    service.task.shutdown().await.unwrap();
}
