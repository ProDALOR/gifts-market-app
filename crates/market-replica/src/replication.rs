//! Snapshot bootstrap, Watch recovery and staged-storage draining.
use crate::{ReplicaWriter, SnapshotJournal, SyncState};
use futures_util::{FutureExt, Stream, StreamExt};
use market_hub_client::HubClient;
use market_hub_protocol::wire as w;
use prost::Message;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::{
    sync::CancellationToken,
    task::{TaskTracker, task_tracker::TaskTrackerToken},
};
use tonic::Status;
mod snapshot;
use snapshot::{SnapshotBatch, SnapshotPages};

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
/// Returned blocking results can wait in async prefetch buffers. Abandoning a
/// large builder/view must release its indexes, database and lease off-thread,
/// and recovery must drain that cleanup before starting another build.
struct StorageValue<T: Send + 'static> {
    value: Option<T>,
    token: Option<TaskTrackerToken>,
    runtime: tokio::runtime::Handle,
}
impl<T: Send + 'static> StorageValue<T> {
    fn new(value: T, jobs: &TaskTracker) -> Self {
        Self {
            value: Some(value),
            token: Some(jobs.token()),
            runtime: tokio::runtime::Handle::current(),
        }
    }
    fn get(&mut self) -> &mut T {
        self.value.as_mut().expect("storage value is present")
    }
    /// Transfer only inside a storage job so cancellation never drops it on async workers.
    fn take(mut self) -> T {
        self.value.take().expect("storage value is present")
    }
}
impl<T: Send + 'static> Drop for StorageValue<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            let token = self.token.take().expect("storage value is tracked");
            self.runtime.spawn_blocking(move || {
                drop(value);
                drop(token);
            });
        }
    }
}
pub(super) async fn collecting<T>(
    work: impl std::future::Future<Output = Result<T, Status>>,
    stream: &mut (impl Stream<Item = Result<w::Batch, Status>> + Unpin),
    journal: &SnapshotJournal,
    queued: &mut u64,
    journal_time: &mut Duration,
    jobs: &TaskTracker,
) -> Result<T, Status> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            result = &mut work => return result,
            batch = stream.next() => {
                let batch=batch.transpose()?.ok_or_else(||Status::unavailable("replication stream closed"))?;
                let started=Instant::now();
                let journal=journal.clone();
                let write=blocking(jobs, move||journal.push(&batch));
                tokio::pin!(write);
                // A staging transaction can hold redb's writer lock. Continue
                // polling page prefetch while this one journal write waits.
                let completed=tokio::select! {
                    result=&mut work=>Some(result),
                    result=&mut write=>{result?;None},
                };
                if let Some(result)=completed {
                    let value=result?;
                    write.await?;
                    *queued+=1;
                    *journal_time+=started.elapsed();
                    return Ok(value);
                }
                *queued+=1;
                *journal_time+=started.elapsed();
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
    batches: u64,
    worker_wait: Duration,
    journal: Duration,
    build: crate::staging::BuildTimings,
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
            batches: 0,
            worker_wait: Duration::ZERO,
            journal: Duration::ZERO,
            build: Default::default(),
        }
    }
    fn log(&mut self, queued: u64, replayed: u64) {
        if self.logged.elapsed() >= Duration::from_secs(5) {
            tracing::info!(
                pages = self.pages,
                rows = self.rows,
                snapshot_batches = self.batches,
                build_state_ms = self.build.state.as_millis(),
                build_db_wait_ms = self.build.db_wait.as_millis(),
                build_db_write_ms = self.build.db_write.as_millis(),
                build_db_commit_ms = self.build.db_commit.as_millis(),
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
    recovery: &mut Recovery,
) -> Result<tonic::Streaming<w::Batch>, Status> {
    let mut progress = BootstrapProgress::new();
    let prepared = replication
        .mutate(jobs, |writer| {
            if matches!(writer.publication, crate::PublicationState::Prepared { .. }) {
                return writer.resume_cursor().map(Some);
            }
            writer.mark(SyncState::Syncing);
            Ok(None)
        })
        .await?;
    if let Some(cursor) = prepared {
        // Publication has completed. A transient Watch setup failure must
        // resume this checkpoint rather than download another full snapshot.
        *recovery = Recovery::Resume;
        return hub.watch(cursor).await;
    }
    tracing::info!("replica snapshot bootstrap started");
    let (first, download) = snapshot_page(hub, String::new()).await?;
    progress.download += download;
    let baseline = first.cursor.clone();
    if baseline.is_empty() {
        return Err(Status::data_loss("missing snapshot checkpoint"));
    }
    let mut changes = hub.watch(baseline.clone()).await?;
    let builder = replication
        .mutate(jobs, |writer| writer.begin_snapshot())
        .await?;
    let journal = builder.journal();
    let mut builder = StorageValue::new(builder, jobs);
    let mut queued = 0;
    let mut pages = SnapshotPages::new(hub, baseline.clone(), first.next_page_token.clone());
    let mut next = SnapshotBatch::first(first)?;
    loop {
        let row_count = next.rows as u64;
        let page_count = next.pages.len() as u64;
        let queued_at = Instant::now();
        let stage = blocking(jobs, move || {
            let wait = queued_at.elapsed();
            let start = Instant::now();
            let rows = next.into_changes();
            builder.get().append(&rows)?;
            drop(rows);
            Ok((builder, start.elapsed(), wait))
        });
        // One bounded batch downloads while another stages; tokens remain sequential.
        let ((prepared, staging, wait), fetched) = collecting(
            async { tokio::try_join!(stage, pages.next_batch()) },
            &mut changes,
            &journal,
            &mut queued,
            &mut progress.journal,
            jobs,
        )
        .await?;
        builder = prepared;
        progress.pages += page_count;
        progress.rows += row_count;
        progress.batches += 1;
        progress.staging += staging;
        progress.worker_wait += wait;
        progress.download = download + pages.download;
        progress.build = builder.get().timings();
        progress.log(queued, 0);
        match fetched {
            Some(batch) => next = batch,
            None => break,
        }
    }
    tracing::info!(
        pages = progress.pages,
        rows = progress.rows,
        queued,
        download_ms = progress.download.as_millis(),
        staging_ms = progress.staging.as_millis(),
        snapshot_batches = progress.batches,
        build_state_ms = progress.build.state.as_millis(),
        build_db_wait_ms = progress.build.db_wait.as_millis(),
        build_db_write_ms = progress.build.db_write.as_millis(),
        build_db_commit_ms = progress.build.db_commit.as_millis(),
        worker_wait_ms = progress.worker_wait.as_millis(),
        journal_ms = progress.journal.as_millis(),
        elapsed_ms = progress.started.elapsed().as_millis(),
        "replica snapshot download completed"
    );
    let mut position = 0;
    let mut cursor = baseline;
    while position < queued {
        let (prepared, replayed, elapsed) = collecting(
            blocking(jobs, move || {
                let start = Instant::now();
                let replayed = builder.get().replay_chunk(position)?;
                Ok((builder, replayed, start.elapsed()))
            }),
            &mut changes,
            &journal,
            &mut queued,
            &mut progress.journal,
            jobs,
        )
        .await?;
        builder = prepared;
        progress.replay += elapsed;
        progress.build = builder.get().timings();
        let replayed =
            replayed.ok_or_else(|| Status::data_loss("missing snapshot journal delivery"))?;
        cursor = replayed.cursor;
        position = replayed.next_position;
        progress.log(queued, position);
    }
    let finalizing = Instant::now();
    let builder_jobs = jobs.clone();
    let snapshot = blocking(jobs, move || {
        let prepared = builder.take().finish(&cursor)?;
        Ok(StorageValue::new(prepared, &builder_jobs))
    })
    .await?;
    let finalized = finalizing.elapsed();
    let publishing = Instant::now();
    replication
        .mutate(jobs, move |writer| writer.install_snapshot(snapshot.take()))
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
        snapshot_batches = progress.batches,
        build_transactions = progress.build.transactions,
        build_state_ms = progress.build.state.as_millis(),
        build_db_wait_ms = progress.build.db_wait.as_millis(),
        build_db_write_ms = progress.build.db_write.as_millis(),
        build_db_commit_ms = progress.build.db_commit.as_millis(),
        worker_wait_ms = progress.worker_wait.as_millis(),
        journal_ms = progress.journal.as_millis(),
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

struct Deliveries {
    batches: Vec<w::Batch>,
    disconnected: Option<Status>,
}
async fn live_deliveries(
    stream: &mut (impl Stream<Item = Result<w::Batch, Status>> + Unpin),
    pending: &mut Option<w::Batch>,
    mut batches: Vec<w::Batch>,
) -> Result<Deliveries, Status> {
    use crate::limits::{MAX_APPLY_BATCHES, MAX_APPLY_BYTES};
    let first = match pending.take() {
        Some(batch) => batch,
        None => stream
            .next()
            .await
            .transpose()?
            .ok_or_else(|| Status::unavailable("replication stream closed"))?,
    };
    let mut bytes = first.encoded_len();
    if bytes > MAX_APPLY_BYTES {
        return Err(Status::resource_exhausted(
            "replication delivery exceeds apply budget",
        ));
    }
    debug_assert!(batches.is_empty());
    batches.push(first);
    let mut deliveries = Deliveries {
        batches,
        disconnected: None,
    };
    // Drain only frames already ready: no timer, collection delay or background
    // reader. Keep one over-budget frame for the following transaction.
    while deliveries.batches.len() < MAX_APPLY_BATCHES && bytes < MAX_APPLY_BYTES {
        match stream.next().now_or_never() {
            Some(Some(Ok(batch))) => {
                let size = batch.encoded_len();
                if size > MAX_APPLY_BYTES - bytes {
                    *pending = Some(batch);
                    break;
                }
                bytes += size;
                deliveries.batches.push(batch);
            }
            Some(ended) => {
                deliveries.disconnected = Some(match ended {
                    Some(Err(error)) => error,
                    _ => Status::unavailable("replication stream closed"),
                });
                break;
            }
            None => break,
        }
    }
    Ok(deliveries)
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
            let stream = bootstrap(replication, hub, jobs, recovery).await?;
            *backoff = 1;
            stream
        }
        Recovery::Resume => {
            // Reconcile a committed-but-unpublished view before requesting the
            // next delivery, including history/cache invalidation after repair.
            let cursor = replication
                .mutate(jobs, |writer| writer.resume_cursor())
                .await?;
            hub.watch(cursor).await?
        }
    };
    *recovery = Recovery::Resume;
    let mut pending = None;
    let mut batches = Vec::new();
    loop {
        let deliveries = live_deliveries(&mut stream, &mut pending, batches).await?;
        batches = replication
            .mutate(jobs, move |writer| {
                let mut batches = deliveries.batches;
                writer.apply_deliveries(&batches)?;
                // Drop decoded payloads on the storage worker, then return just
                // the empty allocation. Cancellation still tracks its ownership.
                batches.clear();
                Ok(batches)
            })
            .await?;
        *backoff = 1;
        if let Some(error) = deliveries.disconnected {
            return Err(error);
        }
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
