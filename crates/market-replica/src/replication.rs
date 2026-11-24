//! Snapshot bootstrap, Watch recovery and staged-storage draining.
use crate::{ReplicaWriter, SnapshotJournal, SyncState};
use futures_util::{FutureExt, Stream, StreamExt};
use market_hub_client::HubClient;
use market_hub_protocol::wire as w;
use prost::Message;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tonic::Status;
mod snapshot;
pub(crate) mod storage_jobs;
use snapshot::{SnapshotBatch, SnapshotPages, SnapshotStats};
use storage_jobs::{StorageJobs, StorageValue};

/// The single upstream worker with permission to submit replica mutations.
pub struct ReplicationWorker {
    pub(crate) writer: Arc<Mutex<ReplicaWriter>>,
}
impl ReplicationWorker {
    async fn mutate<T: Send + 'static>(
        &self,
        jobs: &StorageJobs,
        work: impl FnOnce(&mut ReplicaWriter) -> Result<T, Status> + Send + 'static,
    ) -> Result<T, Status> {
        let writer = self.writer.clone();
        jobs.run(move || {
            let mut writer = writer
                .lock()
                .map_err(|_| Status::unavailable("Replica writer lock poisoned"))?;
            work(&mut writer)
        })
        .await
    }

    pub async fn run(self, hub: HubClient, stop: CancellationToken) -> Result<(), Status> {
        let jobs = StorageJobs::new()?;
        let result = std::panic::AssertUnwindSafe(async {
            let work = run_replication(&self, hub, jobs.clone());
            // A queued storage maintenance job must not suspend snapshot prefetch
            // or Watch journaling while a batch owns the serial storage worker.
            let maintenance = async {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = stop.cancelled() => return Ok::<(), Status>(()),
                        _ = interval.tick() => {},
                    }
                    self.mutate(&jobs, |writer| {
                        writer.poll_persistence()?;
                        writer.expire_views();
                        Ok(())
                    })
                    .await?;
                }
            };
            tokio::select! { biased;
                _ = stop.cancelled() => Ok(()),
                result = work => result,
                result = maintenance => result,
            }
        })
        .catch_unwind()
        .await;
        let flushed = self
            .mutate(&jobs, |writer| writer.finish_persistence())
            .await;
        jobs.shutdown().await;
        self.writer
            .lock()
            .map_err(|_| Status::unavailable("Replica writer lock poisoned"))?
            .mark(SyncState::Stale);
        match result {
            Ok(result) => result.and(flushed),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

pub(super) async fn collecting<T>(
    work: impl std::future::Future<Output = Result<T, Status>>,
    stream: &mut (impl Stream<Item = Result<w::Batch, Status>> + Unpin),
    journal: &SnapshotJournal,
    queued: &mut u64,
    journal_time: &mut Duration,
    jobs: &StorageJobs,
) -> Result<T, Status> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            result = &mut work => return result,
            batch = stream.next() => {
                let batch=batch.transpose()?.ok_or_else(||Status::unavailable("replication stream closed"))?;
                let mut batches = vec![batch];
                let mut bytes = batches[0].encoded_len();
                if bytes > crate::limits::MAX_APPLY_BYTES {
                    return Err(Status::resource_exhausted("replication delivery exceeds journal budget"));
                }
                let mut disconnected = None;
                // One overflow delivery keeps memory below twice the byte budget;
                // each stored record remains independently replayable and bounded.
                while batches.len() < crate::limits::MAX_APPLY_BATCHES && bytes < crate::limits::MAX_APPLY_BYTES {
                    match stream.next().now_or_never() {
                        Some(Some(Ok(batch))) => {
                            let size = batch.encoded_len();
                            if size > crate::limits::MAX_APPLY_BYTES {
                                return Err(Status::resource_exhausted("replication delivery exceeds journal budget"));
                            }
                            bytes += size;
                            batches.push(batch);
                        }
                        Some(ended) => {
                            disconnected = Some(match ended {
                                Some(Err(error)) => error,
                                _ => Status::unavailable("replication stream closed"),
                            });
                            break;
                        }
                        None => break,
                    }
                }
                let count = batches.len() as u64;
                let started=Instant::now();
                let journal=journal.clone();
                let write=jobs.run( move||journal.push_many(&batches));
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
                    *queued+=count;
                    *journal_time+=started.elapsed();
                    if let Some(error) = disconnected { return Err(error); }
                    return Ok(value);
                }
                *queued+=count;
                *journal_time+=started.elapsed();
                if let Some(error) = disconnected { return Err(error); }
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
    snapshot: SnapshotStats,
    journal_stats: crate::staging::JournalStats,
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
            snapshot: Default::default(),
            journal_stats: Default::default(),
        }
    }
    fn log(&mut self, queued: u64, replayed: u64) {
        if self.logged.elapsed() >= Duration::from_secs(5) {
            tracing::info!(
                pages = self.pages,
                rows = self.rows,
                catalog_rows = self.snapshot.catalog,
                feed_rows = self.snapshot.feeds,
                ask_rows = self.snapshot.asks,
                bid_rows = self.snapshot.bids,
                execution_rows = self.snapshot.executions,
                snapshot_bytes = self.snapshot.bytes,
                rows_per_second =
                    self.rows as f64 / self.started.elapsed().as_secs_f64().max(0.001),
                journal_transactions = self.journal_stats.transactions,
                journal_bytes = self.journal_stats.bytes,
                journal_changes = self.journal_stats.changes,
                snapshot_batches = self.batches,
                build_state_ms = self.build.state.as_millis(),
                build_db_wait_ms = self.build.db_wait.as_millis(),
                build_db_write_ms = self.build.db_write.as_millis(),
                build_db_commit_ms = self.build.db_commit.as_millis(),
                history_payload_ms = self.build.history_payload.as_millis(),
                history_indexes_ms = self.build.history_indexes.as_millis(),
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
async fn bootstrap(
    replication: &ReplicationWorker,
    hub: &HubClient,
    jobs: &StorageJobs,
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
    let started = Instant::now();
    let mut source = hub.snapshot().await?;
    let transport = source.transport();
    let first = source
        .next()
        .await?
        .ok_or_else(|| Status::data_loss("missing snapshot page"))?;
    let download = started.elapsed();
    tracing::info!(transport, "replica snapshot transport selected");
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
    let mut pages = SnapshotPages::new(source, baseline.clone());
    let mut next = SnapshotBatch::first(first)?;
    loop {
        let row_count = next.rows as u64;
        let stats = next.stats;
        let page_count = next.pages.len() as u64;
        let queued_at = Instant::now();
        let stage = jobs.run(move || {
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
        progress.snapshot.add(stats);
        progress.journal_stats = journal.stats()?;
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
        catalog_rows = progress.snapshot.catalog,
        feed_rows = progress.snapshot.feeds,
        ask_rows = progress.snapshot.asks,
        bid_rows = progress.snapshot.bids,
        execution_rows = progress.snapshot.executions,
        snapshot_bytes = progress.snapshot.bytes,
        journal_transactions = progress.journal_stats.transactions,
        journal_bytes = progress.journal_stats.bytes,
        journal_changes = progress.journal_stats.changes,
        queued,
        download_ms = progress.download.as_millis(),
        staging_ms = progress.staging.as_millis(),
        snapshot_batches = progress.batches,
        build_state_ms = progress.build.state.as_millis(),
        build_db_wait_ms = progress.build.db_wait.as_millis(),
        build_db_write_ms = progress.build.db_write.as_millis(),
        build_db_commit_ms = progress.build.db_commit.as_millis(),
        history_payload_ms = progress.build.history_payload.as_millis(),
        history_indexes_ms = progress.build.history_indexes.as_millis(),
        worker_wait_ms = progress.worker_wait.as_millis(),
        journal_ms = progress.journal.as_millis(),
        elapsed_ms = progress.started.elapsed().as_millis(),
        "replica snapshot download completed"
    );
    let mut position = 0;
    let mut cursor = baseline;
    while position < queued {
        let (prepared, replayed, elapsed) = collecting(
            jobs.run(move || {
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
        progress.journal_stats = journal.stats()?;
        progress.build = builder.get().timings();
        let replayed =
            replayed.ok_or_else(|| Status::data_loss("missing snapshot journal delivery"))?;
        cursor = replayed.cursor;
        position = replayed.next_position;
        progress.log(queued, position);
    }
    let finalizing = Instant::now();
    let builder_jobs = jobs.clone();
    let snapshot = jobs
        .run(move || {
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
        catalog_rows = progress.snapshot.catalog,
        feed_rows = progress.snapshot.feeds,
        ask_rows = progress.snapshot.asks,
        bid_rows = progress.snapshot.bids,
        execution_rows = progress.snapshot.executions,
        snapshot_bytes = progress.snapshot.bytes,
        journal_transactions = progress.journal_stats.transactions,
        journal_bytes = progress.journal_stats.bytes,
        journal_changes = progress.journal_stats.changes,
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
        history_payload_ms = progress.build.history_payload.as_millis(),
        history_indexes_ms = progress.build.history_indexes.as_millis(),
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
    jobs: &StorageJobs,
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
                writer.apply_owned(deliveries.batches)?;
                Ok(Vec::new())
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
    jobs: StorageJobs,
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
        jobs.drain().await;
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
