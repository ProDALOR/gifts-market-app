//! App-owned read lifetime, shared admission and tracked blocking execution.
use crate::ReplicaReader;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::{
    sync::{CancellationToken, DropGuard},
    task::TaskTracker,
};
use tonic::Status;

/// The single owner cancels and drains all readers, independently of domain actors.
pub struct ReadRuntime {
    handle: ReadExecutor,
}
impl ReadRuntime {
    pub fn new(
        reader: ReplicaReader,
        limit: usize,
        stop: CancellationToken,
    ) -> Result<Self, Status> {
        if limit == 0 || limit > Semaphore::MAX_PERMITS {
            return Err(Status::invalid_argument("Invalid read limit"));
        }
        let clock = tokio::runtime::Handle::try_current()
            .map_err(|_| Status::unavailable("Read runtime requires Tokio"))?;
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        let preparations = cores.clamp(1, 2).min(limit);
        let queries = Arc::new(crate::blocking::BlockingPool::new(
            "market-query",
            cores.clamp(1, 4).min(limit),
        )?);
        let preparation_pool = Arc::new(crate::blocking::BlockingPool::new(
            "market-prepare",
            preparations,
        )?);
        Ok(Self {
            handle: ReadExecutor {
                reader,
                clock,
                budget: ReadBudget {
                    slots: Arc::new(Semaphore::new(limit)),
                    stop: stop.child_token(),
                    limit,
                },
                jobs: TaskTracker::new(),
                queries,
                preparations: preparation_pool,
                workers: Arc::new(Semaphore::new(preparations)),
            },
        })
    }
    pub fn handle(&self) -> ReadExecutor {
        self.handle.clone()
    }
    pub async fn shutdown(&self) {
        self.handle.budget.stop.cancel();
        self.handle.jobs.close();
        self.handle.jobs.wait().await;
        self.handle.queries.shutdown();
        self.handle.preparations.shutdown();
    }
}
impl Drop for ReadRuntime {
    fn drop(&mut self) {
        self.handle.budget.stop.cancel();
    }
}
#[derive(Clone)]
pub struct ReadBudget {
    slots: Arc<Semaphore>,
    stop: CancellationToken,
    limit: usize,
}
/// Queued and running requests retain capacity until their actual work exits.
#[derive(Debug)]
pub struct ReadPermit {
    _slot: OwnedSemaphorePermit,
    cancelled: CancellationToken,
}
impl ReadPermit {
    /// Keep this guard in the requesting future, including while its message is queued.
    pub fn cancel_on_drop(&self) -> DropGuard {
        self.cancelled.clone().drop_guard()
    }
}
impl ReadBudget {
    pub fn limit(&self) -> usize {
        self.limit
    }
    pub async fn cancelled(&self) {
        self.stop.cancelled().await;
    }
    pub fn check(&self) -> Result<(), Status> {
        check(&self.stop)
    }
    pub fn try_query(&self) -> Result<ReadPermit, Status> {
        self.check()?;
        Ok(ReadPermit {
            _slot: self
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| capacity())?,
            cancelled: self.stop.child_token(),
        })
    }
    pub async fn wait(&self) -> Result<ReadPermit, Status> {
        let slot = tokio::select! {biased;
            _=self.stop.cancelled()=>return Err(stopped()),
            permit=self.slots.clone().acquire_owned()=>permit.map_err(|_|capacity())?,
        };
        Ok(ReadPermit {
            _slot: slot,
            cancelled: self.stop.child_token(),
        })
    }
}
fn stopped() -> Status {
    Status::unavailable("Market reads are stopping")
}
fn check(cancelled: &CancellationToken) -> Result<(), Status> {
    if cancelled.is_cancelled() {
        Err(stopped())
    } else {
        Ok(())
    }
}
fn capacity() -> Status {
    Status::resource_exhausted("market query capacity exhausted")
}

#[derive(Clone)]
pub struct ReadExecutor {
    clock: tokio::runtime::Handle,
    reader: ReplicaReader,
    budget: ReadBudget,
    jobs: TaskTracker,
    queries: Arc<crate::blocking::BlockingPool>,
    preparations: Arc<crate::blocking::BlockingPool>,
    workers: Arc<Semaphore>,
}
impl ReadExecutor {
    pub fn budget(&self) -> ReadBudget {
        self.budget.clone()
    }
    pub fn stop_token(&self) -> CancellationToken {
        self.budget.stop.clone()
    }
    /// Live read access for cheap status/revision checks and change notifications.
    pub fn reader(&self) -> ReplicaReader {
        self.reader.with_cancellation(self.stop_token())
    }
    pub fn snapshot(&self) -> Result<ReplicaReader, Status> {
        self.reader().snapshot()
    }
    /// Execute API work already admitted before dispatch to a domain actor.
    pub async fn admitted<T: Send + 'static>(
        &self,
        permit: ReadPermit,
        work: impl FnOnce(ReplicaReader) -> T + Send + 'static,
    ) -> Result<T, Status> {
        let _admission = self.jobs.token();
        let _cancel = permit.cancel_on_drop();
        self.dispatch(permit, None, move |reader, _| work(reader))
            .await
    }
    /// Pin after admission; queued preparation observes the newest published view.
    pub async fn prepare<T: Send + 'static>(
        &self,
        work: impl FnOnce(ReplicaReader, CancellationToken) -> T + Send + 'static,
    ) -> Result<T, Status> {
        self.execute(true, move |reader, cancelled| {
            Ok(work(reader.snapshot()?, cancelled))
        })
        .await?
    }
    /// Small metadata checks share admission without occupying the heavy-read lane.
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
        let queued = std::time::Instant::now();
        self.budget.check()?;
        let cancelled = self.stop_token().child_token();
        let _cancel = cancelled.clone().drop_guard();
        let heavy = if heavy {
            Some(tokio::select! {biased;
                _=cancelled.cancelled()=>return Err(stopped()),
                permit=self.workers.clone().acquire_owned()=>permit.map_err(|_|stopped())?,
            })
        } else {
            None
        };
        let mut permit = self.budget.wait().await?;
        permit.cancelled = cancelled;
        tracing::debug!(
            kind = if heavy.is_some() {
                "preparation"
            } else {
                "metadata"
            },
            admission_ms = queued.elapsed().as_millis(),
            "market read admitted"
        );
        self.dispatch(permit, heavy, work).await
    }
    async fn dispatch<T: Send + 'static>(
        &self,
        permit: ReadPermit,
        heavy: Option<OwnedSemaphorePermit>,
        work: impl FnOnce(ReplicaReader, CancellationToken) -> T + Send + 'static,
    ) -> Result<T, Status> {
        self.budget.check()?;
        if !Arc::ptr_eq(permit._slot.semaphore(), &self.budget.slots) {
            return Err(Status::invalid_argument(
                "Read permit belongs to another runtime",
            ));
        }
        check(&permit.cancelled)?;
        let reader = self.reader.with_cancellation(permit.cancelled.clone());
        let pool = if heavy.is_some() {
            &self.preparations
        } else {
            &self.queries
        };
        let token = self.jobs.token();
        let clock = self.clock.clone();
        pool.spawn(move || {
            let _tracked = token;
            // Execute on the isolated pool, retaining the read owner's clock and
            // async context (not the worker pool's unused current-thread runtime).
            let _context = clock.enter();
            let cancelled = permit.cancelled.clone();
            let (_heavy, _permit) = (heavy, permit);
            check(&cancelled)?;
            Ok(work(reader, cancelled))
        })
        .await
        .map_err(|_| Status::unavailable("Market read worker stopped"))?
    }
}
#[cfg(test)]
mod tests;
