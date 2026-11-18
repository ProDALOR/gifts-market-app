//! Lifecycle of the orderbooks, catalog and user-subscriptions actor group.
use crate::{
    MarketDataHandle,
    catalog::{CatalogActor, handle::CatalogHandle},
    orderbooks::{OrderbooksActor, handle::OrderbooksHandle},
    runtime::ActorEnvironment,
    subscriptions::UserSubscriptionsActor,
};
use actix::Actor;
use market_replica::{ReadExecutor, ReplicaReader};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};
use tokio::{runtime::Handle, sync::Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tonic::Status;
/// Shared actor group used by API and bot consumers.
pub struct MarketDataService {
    pub handle: MarketDataHandle,
    pub task: MarketDataTask,
    pub reads: ReadExecutor,
    arbiter: actix::ArbiterHandle,
}
pub struct MarketDataTask {
    stop: CancellationToken,
    exited: CancellationToken,
    panicked: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    jobs: TaskTracker,
}
impl MarketDataService {
    /// Shared data Arbiter; the application wires independent services onto it.
    pub fn arbiter(&self) -> actix::ArbiterHandle {
        self.arbiter.clone()
    }
    pub async fn start(
        reader: ReplicaReader,
        query_limit: usize,
        subscription_limit: usize,
    ) -> Result<Self, Status> {
        Self::start_with_shutdown(
            reader,
            query_limit,
            subscription_limit,
            CancellationToken::new(),
        )
        .await
    }
    pub async fn start_with_shutdown(
        reader: ReplicaReader,
        query_limit: usize,
        subscription_limit: usize,
        stop: CancellationToken,
    ) -> Result<Self, Status> {
        if query_limit == 0 || subscription_limit == 0 {
            return Err(Status::invalid_argument("Market limits must be positive"));
        }
        let exited = CancellationToken::new();
        let panicked = Arc::new(AtomicBool::new(false));
        let reads = ReadExecutor::new(reader.clone(), query_limit, stop.clone())?;
        let jobs = reads.tasks();
        let runtime = Handle::current();
        let environment = ActorEnvironment {
            jobs: jobs.clone(),
            runtime,
            stop: stop.clone(),
            panicked: panicked.clone(),
        };
        let budget = reads.budget();
        let exit_guard = exited.clone().drop_guard();
        let (ready, started) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("market-data".into())
            .spawn(move || {
                let _guard = exit_guard;
                let system = actix::System::new();
                let actors = system.block_on(async move {
                    let orders = OrderbooksActor {
                        reader: reader.clone(),
                        environment: environment.clone(),
                    }
                    .start();
                    let orderbooks = OrderbooksHandle {
                        address: orders,
                        budget: budget.clone(),
                    };
                    let catalog = CatalogActor {
                        cache: Default::default(),
                        reader: reader.clone(),
                        environment: environment.clone(),
                    }
                    .start();
                    let catalog = CatalogHandle {
                        projections: Arc::new(Semaphore::new((query_limit / 2).clamp(1, 4))),
                        address: catalog,
                        budget,
                    };
                    let subscriptions = UserSubscriptionsActor::new(
                        environment,
                        orderbooks.clone(),
                        catalog.clone(),
                        reader.clone(),
                        subscription_limit,
                    )
                    .start();
                    MarketDataHandle {
                        orderbooks,
                        catalog,
                        subscriptions,
                    }
                });
                if ready.send((actors, actix::Arbiter::current())).is_err() {
                    return;
                }
                let _ = system.run();
            })
            .map_err(|_| Status::unavailable("Market data actor could not start"))?;
        let mut task = MarketDataTask {
            stop: stop.clone(),
            exited,
            panicked,
            thread: Some(thread),
            jobs,
        };
        match started.await {
            Ok((handle, arbiter)) => Ok(Self {
                handle,
                task,
                arbiter,
                reads,
            }),
            Err(_) => {
                task.stop.cancel();
                task.join().await?;
                Err(Status::unavailable("Market data actor failed to start"))
            }
        }
    }
}
impl MarketDataTask {
    async fn join(&mut self) -> Result<(), Status> {
        let result = if let Some(thread) = self.thread.take() {
            tokio::task::spawn_blocking(move || thread.join())
                .await
                .map_err(|_| Status::unavailable("Market data actor join failed"))?
                .map_err(|_| Status::unavailable("Market data actor thread failed"))
        } else {
            Ok(())
        };
        self.jobs.close();
        self.jobs.wait().await;
        result
    }
    pub async fn run(mut self, stop: CancellationToken) -> Result<(), Status> {
        tokio::select! {biased;_=stop.cancelled()=>self.stop.cancel(),_=self.exited.cancelled()=>{}}
        self.join().await?;
        if self.panicked.load(Ordering::Acquire) || !stop.is_cancelled() {
            return Err(Status::unavailable(
                "Market data actor stopped unexpectedly",
            ));
        }
        Ok(())
    }
}
impl Drop for MarketDataTask {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
