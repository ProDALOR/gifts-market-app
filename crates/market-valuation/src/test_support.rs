//! Cross-service fixtures verify shared admission and shutdown with the data actors.
use crate::ValuationSource;
use market_replica::ReplicaReader;
use tonic::Status;
pub struct MarketDataService {
    pub reader: ValuationSource,
    pub handle: market_actors::MarketDataHandle,
    pub task: TestTask,
    arbiter: actix::ArbiterHandle,
}
impl MarketDataService {
    pub async fn start(
        reader: ReplicaReader,
        queries: usize,
        subscriptions: usize,
    ) -> Result<Self, Status> {
        let reads = market_replica::ReadRuntime::new(
            reader,
            queries,
            tokio_util::sync::CancellationToken::new(),
        )?;
        let data = market_actors::MarketDataService::start(reads.handle(), subscriptions).await?;
        Ok(Self {
            arbiter: data.arbiter(),
            reader: ValuationSource::new(reads.handle()),
            handle: data.handle,
            task: TestTask {
                data: data.task,
                reads,
            },
        })
    }
    pub fn arbiter(&self) -> actix::ArbiterHandle {
        self.arbiter.clone()
    }
}

pub struct TestTask {
    data: market_actors::MarketDataTask,
    reads: market_replica::ReadRuntime,
}
impl TestTask {
    pub async fn run(self, stop: tokio_util::sync::CancellationToken) -> Result<(), Status> {
        let result = self.data.run(stop).await;
        self.reads.shutdown().await;
        result
    }
}

/// Advance only the test runtime's monotonic valuation clock, then resume it so
/// bounded waits still allow the actor and blocking executor to make progress.
pub async fn refresh_interval() {
    tokio::time::pause();
    tokio::time::advance(crate::floors::REFRESH_INTERVAL).await;
    tokio::time::resume();
}
