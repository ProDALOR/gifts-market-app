//! Shared bounded blocking reads. Permits and tracking outlive cancelled callers.
use crate::ReplicaReader;
use std::sync::Arc;
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tonic::Status;
#[derive(Clone)]
pub struct ReadBudget {
    pub(crate) slots: Arc<Semaphore>,
    pub(crate) stop: CancellationToken,
}
impl ReadBudget {
    pub async fn cancelled(&self) {
        self.stop.cancelled().await;
    }
    pub fn check(&self) -> Result<(), Status> {
        if self.stop.is_cancelled() {
            Err(Status::unavailable("Market data service is stopping"))
        } else {
            Ok(())
        }
    }
    pub fn try_query(&self) -> Result<OwnedSemaphorePermit, Status> {
        self.check()?;
        self.slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| capacity())
    }
    pub async fn wait(&self) -> Result<OwnedSemaphorePermit, Status> {
        tokio::select! {biased;_=self.stop.cancelled()=>Err(Status::unavailable("Market data service stopped")),permit=self.slots.clone().acquire_owned()=>permit.map_err(|_|capacity())}
    }
}
fn capacity() -> Status {
    Status::resource_exhausted("market query capacity exhausted")
}

#[derive(Clone)]
pub struct ReadExecutor {
    reader: ReplicaReader,
    budget: ReadBudget,
    jobs: TaskTracker,
    runtime: Handle,
    workers: Arc<Semaphore>,
}
impl ReadExecutor {
    pub fn new(
        reader: ReplicaReader,
        limit: usize,
        stop: CancellationToken,
    ) -> Result<Self, Status> {
        if limit == 0 {
            return Err(Status::invalid_argument("Read limit must be positive"));
        }
        Ok(Self {
            reader,
            budget: ReadBudget {
                slots: Arc::new(Semaphore::new(limit)),
                stop,
            },
            jobs: TaskTracker::new(),
            runtime: Handle::current(),
            workers: Arc::new(Semaphore::new(1)),
        })
    }
    pub fn budget(&self) -> ReadBudget {
        self.budget.clone()
    }
    pub fn tasks(&self) -> TaskTracker {
        self.jobs.clone()
    }
    pub fn stop_token(&self) -> CancellationToken {
        self.budget.stop.clone()
    }
    pub fn snapshot(&self) -> Result<ReplicaReader, Status> {
        self.budget.check()?;
        self.reader.snapshot()
    }
    /// Pin only after admission, so queued reads use the newest published view.
    pub async fn prepare<T: Send + 'static>(
        &self,
        work: impl FnOnce(ReplicaReader, CancellationToken) -> T + Send + 'static,
    ) -> Result<T, Status> {
        self.execute(true, move |reader, cancelled| {
            Ok(work(reader.snapshot()?, cancelled))
        })
        .await?
    }
    /// Small metadata validation shares admission without blocking the heavy-read lane.
    pub async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(CancellationToken) -> T + Send + 'static,
    ) -> Result<T, Status> {
        self.execute(false, move |_, cancelled| work(cancelled))
            .await
    }
    async fn execute<T: Send + 'static>(
        &self,
        heavy: bool,
        work: impl FnOnce(ReplicaReader, CancellationToken) -> T + Send + 'static,
    ) -> Result<T, Status> {
        let _admission = self.jobs.token();
        self.budget.check()?;
        let cancelled = self.budget.stop.child_token();
        let _cancel = cancelled.clone().drop_guard();
        let heavy = if heavy {
            Some(tokio::select! {biased;
                _=cancelled.cancelled()=>return Err(Status::unavailable("Market data service is stopping")),
                permit=self.workers.clone().acquire_owned()=>permit.map_err(|_|Status::unavailable("Read workers closed"))?,
            })
        } else {
            None
        };
        let permit = self.budget.wait().await?;
        let reader = self.reader.clone();
        self.jobs
            .spawn_blocking_on(
                move || {
                    let (_heavy, _permit) = (heavy, permit);
                    work(reader, cancelled)
                },
                &self.runtime,
            )
            .await
            .map_err(|_| Status::unavailable("Market read worker stopped"))
    }
}

#[cfg(test)]
mod tests;
