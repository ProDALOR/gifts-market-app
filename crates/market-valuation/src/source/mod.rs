//! Gift-specific read projections over the generic replica and its shared executor.
mod catalog;
mod dependencies;
mod gift;
mod inventory;
mod orders;
pub use catalog::CatalogTrait;
pub(crate) use dependencies::METADATA_KEY_LIMIT;
pub(crate) use dependencies::OrderDependency;
pub use dependencies::ReadDependencies;
use market_replica::{ReadExecutor, ReplicaReader};
pub(crate) use orders::AskBookSnapshot;
pub use orders::BookScope;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tonic::Status;
type Result<T> = std::result::Result<T, Status>;
#[derive(Clone)]
pub struct ValuationSource {
    executor: ReadExecutor,
}
#[derive(Clone)]
pub struct ValuationSnapshot {
    reader: ReplicaReader,
    cancelled: CancellationToken,
}
impl ValuationSource {
    #[cfg(test)]
    pub(crate) fn executor(&self) -> ReadExecutor {
        self.executor.clone()
    }
    pub fn new(executor: ReadExecutor) -> Self {
        Self { executor }
    }
    pub fn snapshot(&self) -> Result<ValuationSnapshot> {
        Ok(ValuationSnapshot {
            reader: self.executor.snapshot()?,
            cancelled: self.executor.stop_token().child_token(),
        })
    }
    pub async fn prepare<T: Send + 'static>(
        &self,
        work: impl FnOnce(ValuationSnapshot) -> T + Send + 'static,
    ) -> Result<T> {
        self.executor
            .prepare(move |reader, cancelled| work(ValuationSnapshot { reader, cancelled }))
            .await
    }
}
impl ValuationSnapshot {
    pub fn check_cancelled(&self) -> Result<()> {
        if self.cancelled.is_cancelled() {
            Err(Status::unavailable("Market read cancelled"))
        } else {
            Ok(())
        }
    }
    pub(crate) fn generation(&self) -> Result<u64> {
        self.reader.generation()
    }
    pub fn revision(&self) -> Result<String> {
        self.check_cancelled()?;
        self.reader.cursor()
    }
    pub fn sync(&self) -> Result<market_gifts::SyncStatus> {
        self.check_cancelled()?;
        let status = self.reader.status();
        Ok(market_gifts::SyncStatus {
            ready: status.ready,
            state: match status.state {
                market_replica::SyncState::Starting => market_gifts::SyncState::Starting,
                market_replica::SyncState::Syncing => market_gifts::SyncState::Syncing,
                market_replica::SyncState::Live => market_gifts::SyncState::Live,
                market_replica::SyncState::Stale => market_gifts::SyncState::Stale,
            },
            updated_at: status
                .updated_at
                .map(jiff::Timestamp::from_millisecond)
                .transpose()
                .map_err(|e| Status::data_loss(e.to_string()))?,
        })
    }
}
#[cfg(test)]
mod tests;

/// Internal bounded traversal result used by strategy preparation.
pub struct Page<T> {
    pub data: Vec<T>,
    pub next_page_token: String,
}
