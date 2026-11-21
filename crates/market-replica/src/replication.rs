//! Snapshot bootstrap, Watch recovery and staged-storage draining.
use crate::{ReplicaWriter, SnapshotJournal, SyncState};
use futures_util::FutureExt;
use market_hub_client::HubClient;
use market_hub_protocol::wire as w;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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
            let work=run_replication(&self,hub,jobs.clone());
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
        drain(&jobs).await;
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
    // Track the result's ownership too: cancellation may abandon a returned
    // builder before the blocking task's output is destroyed. Tuple fields drop
    // in order, so draining waits until that output has released its build lease.
    let token = jobs.token();
    let (result, _token) = tokio::task::spawn_blocking(move || (work(), token))
        .await
        .map_err(|_| Status::unavailable("Replica storage worker stopped"))?;
    result
}
async fn drain(jobs: &TaskTracker) {
    jobs.close();
    jobs.wait().await;
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
struct BootstrapProgress {
    started: Instant,
    logged: Instant,
    pages: u64,
    rows: u64,
    download: Duration,
    staging: Duration,
    replay: Duration,
}
impl BootstrapProgress {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            logged: Instant::now(),
            pages: 0,
            rows: 0,
            download: Duration::ZERO,
            staging: Duration::ZERO,
            replay: Duration::ZERO,
        }
    }
    fn log(&mut self, queued: u64, replayed: u64) {
        if self.logged.elapsed() >= Duration::from_secs(5) {
            tracing::info!(
                pages = self.pages,
                rows = self.rows,
                queued,
                replayed,
                backlog = queued.saturating_sub(replayed),
                elapsed_ms = self.started.elapsed().as_millis(),
                "replica snapshot bootstrap progress"
            );
            self.logged = Instant::now();
        }
    }
}
async fn snapshot_page(hub: &HubClient, token: String) -> Result<(w::Page, Duration), Status> {
    let start = Instant::now();
    let page = hub.snapshot_page(token).await?;
    Ok((page, start.elapsed()))
}
async fn bootstrap(
    replication: &ReplicationWorker,
    hub: &HubClient,
    jobs: &TaskTracker,
) -> Result<tonic::Streaming<w::Batch>, Status> {
    let mut progress = BootstrapProgress::new();
    replication
        .mutate(jobs, |writer| {
            writer.mark(SyncState::Syncing);
            Ok(())
        })
        .await?;
    tracing::info!("replica snapshot bootstrap started");
    let (mut next, download) = snapshot_page(hub, String::new()).await?;
    progress.download += download;
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
        progress.rows += next.entities.len() as u64;
        let entities = next.entities;
        let stage = blocking(jobs, move || {
            let start = Instant::now();
            let rows = entities
                .into_iter()
                .map(|entity| w::Change {
                    change: Some(w::change::Change::Upsert(entity)),
                })
                .collect::<Vec<_>>();
            builder.append(&rows)?;
            Ok((builder, start.elapsed()))
        });
        // Continuations remain sequential, but one next page may download while
        // this page builds indexes and commits to the unpublished staging store.
        let fetch = async {
            if token.is_empty() {
                Ok(None)
            } else {
                snapshot_page(hub, token).await.map(Some)
            }
        };
        let ((prepared, staging), fetched) = collecting(
            async { tokio::try_join!(stage, fetch) },
            &mut changes,
            &journal,
            &mut queued,
            jobs,
        )
        .await?;
        builder = prepared;
        progress.pages += 1;
        progress.staging += staging;
        progress.log(queued, 0);
        match fetched {
            Some((page, download)) => {
                next = page;
                progress.download += download;
            }
            None => break,
        }
    }
    tracing::info!(
        pages = progress.pages,
        rows = progress.rows,
        queued,
        download_ms = progress.download.as_millis(),
        staging_ms = progress.staging.as_millis(),
        elapsed_ms = progress.started.elapsed().as_millis(),
        "replica snapshot download completed"
    );
    let mut position = 0;
    let mut cursor = baseline;
    while position < queued {
        let (prepared, replayed, elapsed) = collecting(
            blocking(jobs, move || {
                let start = Instant::now();
                let replayed = builder.replay_chunk(position)?;
                Ok((builder, replayed, start.elapsed()))
            }),
            &mut changes,
            &journal,
            &mut queued,
            jobs,
        )
        .await?;
        builder = prepared;
        progress.replay += elapsed;
        let replayed =
            replayed.ok_or_else(|| Status::data_loss("missing snapshot journal delivery"))?;
        cursor = replayed.cursor;
        position = replayed.next_position;
        progress.log(queued, position);
    }
    let finalizing = Instant::now();
    let snapshot = blocking(jobs, move || builder.finish(&cursor)).await?;
    let finalized = finalizing.elapsed();
    let publishing = Instant::now();
    replication
        .mutate(jobs, move |writer| writer.install_snapshot(snapshot))
        .await?;
    tracing::info!(
        pages = progress.pages,
        rows = progress.rows,
        replayed = position,
        download_ms = progress.download.as_millis(),
        staging_ms = progress.staging.as_millis(),
        replay_ms = progress.replay.as_millis(),
        finalize_ms = finalized.as_millis(),
        publish_ms = publishing.elapsed().as_millis(),
        elapsed_ms = progress.started.elapsed().as_millis(),
        "replica snapshot bootstrap completed"
    );
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
        Recovery::Bootstrap => {
            let stream = bootstrap(replication, hub, jobs).await?;
            *backoff = 1;
            stream
        }
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
        let error = match session(replication, &hub, &jobs, &mut recovery, &mut backoff).await {
            Ok(()) => break,
            Err(error) => error,
        };
        // A failed network future can leave blocking staging/replay work behind.
        // Release its builder/results before another session can acquire a lease
        // or mutate the published replica. Shutdown drains the same tracker.
        drain(&jobs).await;
        jobs.reopen();
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
        tokio::time::sleep(delay).await;
        backoff = (backoff * 2).min(30);
    }
    Ok(())
}

#[cfg(test)]
#[path = "replication_tests.rs"]
mod tests;
