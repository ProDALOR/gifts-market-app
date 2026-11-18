//! Shared lifecycle, bounded read workers and admission across actor responsibilities.
use actix::prelude::*;
use market_replica::ReplicaReader;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::{runtime::Handle, sync::OwnedSemaphorePermit};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tonic::Status;
#[derive(Clone)]
pub(crate) struct ActorEnvironment {
    pub jobs: TaskTracker,
    pub runtime: Handle,
    pub stop: CancellationToken,
    pub panicked: Arc<AtomicBool>,
}
impl ActorEnvironment {
    pub fn started<A: Actor<Context = Context<A>>>(&self, ctx: &mut Context<A>) {
        ctx.set_mailbox_capacity(32);
        let stop = self.stop.clone();
        ctx.spawn(
            actix::fut::wrap_future::<_, A>(async move { stop.cancelled().await })
                .map(|_, _, ctx| ctx.stop()),
        );
    }
    pub fn stopped(&self) {
        if std::thread::panicking() {
            self.panicked.store(true, Ordering::Release);
        }
        self.stop.cancel();
        if let Some(system) = actix::System::try_current() {
            system.stop();
        }
    }
    pub fn blocking<T: Send + 'static>(
        &self,
        reader: ReplicaReader,
        permit: OwnedSemaphorePermit,
        work: impl FnOnce(ReplicaReader) -> Result<T, Status> + Send + 'static,
    ) -> ResponseFuture<Result<T, Status>> {
        let job = self.jobs.spawn_blocking_on(
            move || {
                let _permit = permit;
                work(reader)
            },
            &self.runtime,
        );
        Box::pin(async move {
            job.await
                .map_err(|_| Status::unavailable("Market read worker stopped"))?
        })
    }
}
pub(crate) use market_replica::ReadBudget as QueryBudget;
pub(crate) fn closed(_: actix::MailboxError) -> Status {
    Status::unavailable("Market data actor is unavailable")
}

pub(crate) fn capacity() -> Status {
    Status::resource_exhausted("market query capacity exhausted")
}
