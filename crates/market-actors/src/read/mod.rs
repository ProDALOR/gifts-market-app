//! Read-only market access and tracked execution for independent consumers.
mod catalog;
mod dependencies;
mod execution;
mod orders;
pub use crate::catalog::ResolvedModel;
use crate::{
    error::{DataError, Result},
    runtime::{ActorEnvironment, QueryBudget},
};
pub use catalog::CatalogTrait;
pub use dependencies::ReadDependencies;
pub use market_gifts::AskQuote;
use market_replica::ReplicaReader;
pub use orders::BookScope;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub struct MarketReader {
    reader: ReplicaReader,
    environment: ActorEnvironment,
    budget: QueryBudget,
    pub(crate) workers: Arc<Semaphore>,
}
#[derive(Clone)]
pub struct MarketSnapshot {
    reader: ReplicaReader,
    cancelled: CancellationToken,
}
impl MarketReader {
    pub(crate) fn new(
        reader: ReplicaReader,
        environment: ActorEnvironment,
        budget: QueryBudget,
    ) -> Self {
        Self {
            reader,
            environment,
            budget,
            workers: Arc::new(Semaphore::new(1)),
        }
    }
    pub fn snapshot(&self) -> Result<MarketSnapshot> {
        if self.environment.stop.is_cancelled() {
            return Err(DataError::unavailable("Market data service is stopping"));
        }
        Ok(MarketSnapshot {
            reader: self.reader.snapshot()?,
            cancelled: self.environment.stop.child_token(),
        })
    }
}
impl MarketSnapshot {
    pub fn check_cancelled(&self) -> Result<()> {
        if self.cancelled.is_cancelled() {
            Err(DataError::unavailable("Market read cancelled"))
        } else {
            Ok(())
        }
    }
    pub fn revision(&self) -> Result<String> {
        self.check_cancelled()?;
        Ok(self.reader.cursor()?)
    }
    pub fn sync(&self) -> Result<market_gifts::SyncStatus> {
        self.check_cancelled()?;
        crate::protocol::sync(self.reader.status())
    }
}

#[cfg(test)]
mod tests;
