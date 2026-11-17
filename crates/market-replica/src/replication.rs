//! Snapshot bootstrap, Watch recovery and staged-storage draining.
use crate::{ReplicaWriter, SnapshotJournal, SyncState};
use futures_util::FutureExt;
use market_hub_client::HubClient;
use market_hub_protocol::wire as w;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tonic::Status;

/// The single upstream worker with permission to submit replica mutations.
pub struct ReplicationWorker {
    pub(crate) writer: Arc<Mutex<ReplicaWriter>>,
}
impl ReplicationWorker {
    async fn mutate<T: Send + 'static>(
        &self,
        jobs: &TaskTracker,
        work: impl FnOnce(&mut ReplicaWriter) -> Result<T, Status> + Send + 'static,
    ) -> Result<T, Status> {
        let writer = self.writer.clone();
        blocking(jobs, move || {
            let mut writer = writer
                .lock()
                .map_err(|_| Status::unavailable("Replica writer lock poisoned"))?;
            work(&mut writer)
        })
        .await
    }

    pub async fn run(self, hub: HubClient, stop: CancellationToken) -> Result<(), Status> {
        let jobs = TaskTracker::new();
        let result = std::panic::AssertUnwindSafe(async {
            let work=run_replication(&self,hub,stop.clone(),jobs.clone());
            tokio::pin!(work);
            let mut maintenance=tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    biased;
                    _=stop.cancelled()=>break Ok(()),
                    result=&mut work=>break result,
                    _=maintenance.tick()=>self.mutate(&jobs,|writer|{writer.expire_views();Ok(())}).await?,
                }
            }
        })
        .catch_unwind()
        .await;
        jobs.close();
        jobs.wait().await;
        self.writer
            .lock()
            .map_err(|_| Status::unavailable("Replica writer lock poisoned"))?
            .mark(SyncState::Stale);
        match result {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

async fn blocking<T: Send + 'static>(
    jobs: &TaskTracker,
    work: impl FnOnce() -> Result<T, Status> + Send + 'static,
) -> Result<T, Status> {
    jobs.spawn_blocking(work)
        .await
        .map_err(|_| Status::unavailable("Replica storage worker stopped"))?
}
async fn collecting<T>(
    work: impl std::future::Future<Output = Result<T, Status>>,
    stream: &mut tonic::Streaming<w::Batch>,
    journal: &SnapshotJournal,
    queued: &mut u64,
    jobs: &TaskTracker,
) -> Result<T, Status> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            result = &mut work => return result,
            batch = stream.message() => {
                let batch=batch?.ok_or_else(||Status::unavailable("replication stream closed"))?;
                let journal=journal.clone();blocking(jobs, move||journal.push(&batch)).await?;*queued+=1;
            }
        }
    }
}
async fn bootstrap(
    replication: &ReplicationWorker,
    hub: &HubClient,
    jobs: &TaskTracker,
) -> Result<tonic::Streaming<w::Batch>, Status> {
    replication
        .mutate(jobs, |writer| {
            writer.mark(SyncState::Syncing);
            Ok(())
        })
        .await?;
    let mut next = hub.snapshot_page(String::new()).await?;
    let baseline = next.cursor.clone();
    if baseline.is_empty() {
        return Err(Status::data_loss("missing snapshot checkpoint"));
    }
    let mut changes = hub.watch(baseline.clone()).await?;
    let mut builder = replication
        .mutate(jobs, |writer| writer.begin_snapshot())
        .await?;
    let journal = builder.journal();
    let mut queued = 0;
    loop {
        if next.cursor != baseline {
            return Err(Status::data_loss("snapshot checkpoint changed"));
        }
        let token = next.next_page_token;
        let rows = next
            .entities
            .into_iter()
            .map(|entity| w::Change {
                change: Some(w::change::Change::Upsert(entity)),
            })
            .collect::<Vec<_>>();
        builder = collecting(
            blocking(jobs, move || {
                builder.append(&rows)?;
                Ok(builder)
            }),
            &mut changes,
            &journal,
            &mut queued,
            jobs,
        )
        .await?;
        if token.is_empty() {
            break;
        }
        next = collecting(
            hub.snapshot_page(token),
            &mut changes,
            &journal,
            &mut queued,
            jobs,
        )
        .await?;
    }
    // Replay one bounded delivery at a time while continuing to drain Watch to disk.
    let mut position = 0;
    let mut cursor = baseline;
    loop {
        let (next_builder, checkpoint) = collecting(
            blocking(jobs, move || {
                let checkpoint = builder.replay(position)?;
                Ok((builder, checkpoint))
            }),
            &mut changes,
            &journal,
            &mut queued,
            jobs,
        )
        .await?;
        builder = next_builder;
        if let Some(checkpoint) = checkpoint {
            cursor = checkpoint;
            position += 1;
        } else if position == queued {
            break;
        }
    }
    let snapshot = blocking(jobs, move || builder.finish(&cursor)).await?;
    replication
        .mutate(jobs, move |writer| writer.install_snapshot(snapshot))
        .await?;
    Ok(changes)
}
#[derive(Clone, Copy, Debug)]
enum Recovery {
    Bootstrap,
    Resume,
}

async fn session(
    replication: &ReplicationWorker,
    hub: &HubClient,
    jobs: &TaskTracker,
    recovery: &mut Recovery,
    backoff: &mut u64,
) -> Result<(), Status> {
    let mut stream = match recovery {
        Recovery::Bootstrap => bootstrap(replication, hub, jobs).await?,
        Recovery::Resume => {
            let cursor = replication.mutate(jobs, |writer| writer.cursor()).await?;
            hub.watch(cursor).await?
        }
    };
    *recovery = Recovery::Resume;
    loop {
        let batch = stream
            .message()
            .await?
            .ok_or_else(|| Status::unavailable("replication stream closed"))?;
        replication
            .mutate(jobs, move |writer| writer.apply(batch))
            .await?;
        *backoff = 1;
    }
}
async fn run_replication(
    replication: &ReplicationWorker,
    hub: HubClient,
    stop: CancellationToken,
    jobs: TaskTracker,
) -> Result<(), Status> {
    let needs_snapshot = replication
        .mutate(&jobs, |writer| writer.cursor())
        .await?
        .is_empty();
    let mut recovery = if needs_snapshot {
        Recovery::Bootstrap
    } else {
        Recovery::Resume
    };
    let mut backoff = 1;
    loop {
        let error = tokio::select! {
            _=stop.cancelled()=>break,
            result=session(replication,&hub,&jobs,&mut recovery,&mut backoff)=>match result{Ok(())=>break,Err(error)=>error},
        };
        if matches!(
            error.code(),
            tonic::Code::FailedPrecondition
                | tonic::Code::OutOfRange
                | tonic::Code::InvalidArgument
        ) {
            recovery = Recovery::Bootstrap;
        }
        tracing::warn!(code=?error.code(), message=error.message(), recovery=?recovery, "replica disconnected; reconnecting");
        replication
            .mutate(&jobs, |writer| {
                writer.mark(SyncState::Stale);
                Ok(())
            })
            .await?;
        let delay =
            Duration::from_millis(backoff * 1000 + (uuid::Uuid::new_v4().as_u128() % 251) as u64);
        tokio::select! {_=stop.cancelled()=>break,_=tokio::time::sleep(delay)=>{}}
        backoff = (backoff * 2).min(30);
    }
    Ok(())
}

#[cfg(test)]
#[path = "replication_tests.rs"]
mod tests;
