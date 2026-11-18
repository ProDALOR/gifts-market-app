//! Cross-service fixtures verify shared admission and shutdown with the data actors.
use crate::ValuationSource;
use market_replica::ReplicaReader;
use tonic::Status;
pub struct MarketDataService {
    pub reader: ValuationSource,
    pub handle: market_actors::MarketDataHandle,
    pub task: market_actors::MarketDataTask,
    arbiter: actix::ArbiterHandle,
}
impl MarketDataService {
    pub async fn start(
        reader: ReplicaReader,
        queries: usize,
        subscriptions: usize,
    ) -> Result<Self, Status> {
        let data = market_actors::MarketDataService::start(reader, queries, subscriptions).await?;
        Ok(Self {
            arbiter: data.arbiter(),
            reader: ValuationSource::new(data.reads),
            handle: data.handle,
            task: data.task,
        })
    }
    pub fn arbiter(&self) -> actix::ArbiterHandle {
        self.arbiter.clone()
    }
}
