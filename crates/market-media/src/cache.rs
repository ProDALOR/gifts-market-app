use crate::{
    MediaAsset, MediaError, MediaKey, MediaLimits, MediaResult,
    disk::DiskStore,
    scheduler::{Job, MediaDownloader, Queue},
};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};
use tokio_util::task::TaskTracker;
struct Resident {
    asset: MediaAsset,
    stamp: u64,
    expires: Option<Instant>,
}
#[derive(Default)]
struct Memory {
    assets: HashMap<MediaKey, Resident>,
    bytes: usize,
    clock: u64,
}
#[derive(Default)]
pub(crate) struct Counters {
    pub hits: AtomicU64,
    pub reads: AtomicU64,
    pub downloads: AtomicU64,
    pub skipped: AtomicU64,
    pub renders: AtomicU64,
    pub covers: AtomicU64,
}
#[derive(Debug, Clone, Copy)]
pub struct MediaStats {
    pub memory_hits: u64,
    pub disk_reads: u64,
    pub downloads: u64,
    pub skipped_downloads: u64,
    pub preview_generations: u64,
    pub cover_generations: u64,
    pub memory_bytes: usize,
    pub queued: usize,
}
pub(crate) struct Shared {
    pub limits: MediaLimits,
    pub queue: Arc<Queue>,
    pub counters: Counters,
    disk: Arc<DiskStore>,
    memory: Mutex<Memory>,
    pending: Mutex<HashMap<MediaKey, Weak<Job>>>,
    failures: Mutex<HashMap<MediaKey, (Instant, MediaError)>>,
    pub(crate) tasks: TaskTracker,
    io_slots: Arc<tokio::sync::Semaphore>,
    pub(crate) admission: Mutex<()>,
    pub(crate) closed: AtomicBool,
    pub(crate) stop: tokio_util::sync::CancellationToken,
    pub(crate) preview_slots: Arc<tokio::sync::Semaphore>,
    pub(crate) renderer: std::path::PathBuf,
}
#[derive(Clone)]
pub struct MediaCache(pub(crate) Arc<Shared>);
impl MediaCache {
    pub async fn open(directory: impl AsRef<Path>) -> std::io::Result<(Self, MediaDownloader)> {
        Self::with_limits(directory, MediaLimits::default()).await
    }
    pub async fn with_limits(
        directory: impl AsRef<Path>,
        limits: MediaLimits,
    ) -> std::io::Result<(Self, MediaDownloader)> {
        let renderer = std::env::current_exe()?.with_file_name("market-media-render");
        Self::with_renderer(directory, limits, renderer).await
    }
    pub(crate) async fn with_renderer(
        directory: impl AsRef<Path>,
        limits: MediaLimits,
        renderer: std::path::PathBuf,
    ) -> std::io::Result<(Self, MediaDownloader)> {
        limits.validate()?;
        let directory = directory.as_ref().to_owned();
        let disk_limits = limits.clone();
        let disk = tokio::task::spawn_blocking(move || DiskStore::open(directory, disk_limits))
            .await
            .map_err(std::io::Error::other)??;
        let cache = Self(Arc::new(Shared {
            limits,
            disk: Arc::new(disk),
            queue: Arc::new(Queue::default()),
            counters: Counters::default(),
            memory: Mutex::default(),
            pending: Mutex::default(),
            failures: Mutex::default(),
            tasks: TaskTracker::new(),
            io_slots: Arc::new(tokio::sync::Semaphore::new(4)),
            admission: Mutex::new(()),
            closed: AtomicBool::new(false),
            stop: tokio_util::sync::CancellationToken::new(),
            preview_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            renderer,
        }));
        Ok((cache.clone(), MediaDownloader::new(cache)))
    }
    pub(crate) fn resident(&self, key: MediaKey) -> Option<MediaAsset> {
        let mut memory = self.0.memory.lock().unwrap_or_else(|e| e.into_inner());
        memory.clock += 1;
        let stamp = memory.clock;
        if memory.assets.get(&key).is_some_and(|entry| {
            entry
                .expires
                .is_some_and(|expires| expires <= Instant::now())
        }) && let Some(old) = memory.assets.remove(&key)
        {
            memory.bytes -= old.asset.bytes().len();
        }
        let asset = memory.assets.get_mut(&key).map(|entry| {
            entry.stamp = stamp;
            entry.asset.clone()
        });
        if asset.is_some() {
            self.0.disk.touch(key);
            self.0.counters.hits.fetch_add(1, Ordering::Relaxed);
        }
        asset
    }
    pub(crate) fn remember(&self, key: MediaKey, asset: &MediaAsset) {
        let mut memory = self.0.memory.lock().unwrap_or_else(|e| e.into_inner());
        // An older read/write may finish after a replacement has reached disk.
        if key.kind() == crate::MediaKind::Animation
            && !self.0.disk.current_source(key.document(), asset.digest)
        {
            return;
        }
        if let Some(old) = memory.assets.remove(&key) {
            memory.bytes -= old.asset.bytes().len();
        }
        if asset.bytes().len() > self.0.limits.memory_bytes {
            return;
        }
        while memory.bytes + asset.bytes().len() > self.0.limits.memory_bytes {
            let Some(old) = memory
                .assets
                .iter()
                .min_by_key(|(_, value)| value.stamp)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(old) = memory.assets.remove(&old) {
                memory.bytes -= old.asset.bytes().len();
            }
        }
        memory.clock += 1;
        let stamp = memory.clock;
        memory.bytes += asset.bytes().len();
        memory.assets.insert(
            key,
            Resident {
                asset: asset.clone(),
                stamp,
                // Derivatives are immutable by source hash and renderer version. Source
                // changes select a new key; only originals need periodic revalidation.
                expires: (!matches!(
                    key.kind(),
                    crate::MediaKind::Preview | crate::MediaKind::Cover
                ))
                .then(|| Instant::now() + self.0.limits.memory_ttl),
            },
        );
    }
    pub(crate) fn job(&self, key: MediaKey) -> Result<Arc<Job>, MediaError> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err(MediaError::Unavailable);
        }
        let mut pending = self.0.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|_, job| job.strong_count() > 0);
        if let Some(job) = pending.get(&key).and_then(Weak::upgrade) {
            return Ok(job);
        }
        if pending.len() >= self.0.limits.queue {
            return Err(MediaError::Busy);
        }
        let job = Arc::new(Job::new(key));
        pending.insert(key, Arc::downgrade(&job));
        Ok(job)
    }
    pub(crate) fn retire(&self, job: &Arc<Job>) {
        let mut pending = self.0.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending
            .get(&job.key)
            .is_some_and(|old| old.ptr_eq(&Arc::downgrade(job)))
        {
            pending.remove(&job.key);
        }
    }
    pub(crate) async fn disk(&self, job: &Job) -> Result<Option<MediaAsset>, MediaError> {
        job.disk
            .get_or_init(|| async {
                self.0.counters.reads.fetch_add(1, Ordering::Relaxed);
                let disk = self.0.disk.clone();
                let key = job.key;
                let result = self.blocking(move || disk.read(key)).await?;
                if let Some(asset) = &result {
                    self.remember(key, asset);
                }
                Ok(result)
            })
            .await
            .clone()
    }
    pub async fn cached(&self, key: MediaKey) -> Result<Option<MediaAsset>, MediaError> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err(MediaError::Unavailable);
        }
        if let Some(asset) = self.resident(key) {
            return Ok(Some(asset));
        }
        let job = self.job(key)?;
        self.disk(&job).await
    }
    pub async fn get(&self, key: MediaKey) -> MediaResult {
        if matches!(
            key.kind(),
            crate::MediaKind::Preview | crate::MediaKind::Cover
        ) {
            return Err(MediaError::InvalidMedia);
        }
        if self.0.closed.load(Ordering::Acquire) {
            return Err(MediaError::Unavailable);
        }
        if let Some(asset) = self.resident(key) {
            return Ok(asset);
        }
        let job = self.job(key)?;
        let _demand = job.demand();
        if let Some(asset) = self.disk(&job).await? {
            return Ok(asset);
        }
        if let Some(error) = self.failure(key) {
            return Err(error);
        }
        if !job.queued.swap(true, Ordering::AcqRel)
            && !self.0.queue.push(job.clone(), self.0.limits.queue)
        {
            job.result.send_replace(Some(Err(MediaError::Unavailable)));
            self.retire(&job);
        }
        job.wait(self.0.limits.request_timeout).await
    }
    pub(crate) async fn derive<F, Fut>(&self, key: MediaKey, build: F) -> MediaResult
    where
        F: FnOnce(Self, Arc<Job>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = MediaResult> + Send,
    {
        use crate::{MediaKind, scheduler::Running};
        if self.0.closed.load(Ordering::Acquire) {
            return Err(MediaError::Unavailable);
        }
        if !matches!(key.kind(), MediaKind::Preview | MediaKind::Cover) {
            return Err(MediaError::InvalidMedia);
        }
        if let Some(asset) = self.resident(key) {
            return Ok(asset);
        }
        let job = self.job(key)?;
        let _demand = job.demand();
        if let Some(asset) = self.disk(&job).await? {
            return Ok(asset);
        }
        if let Some(error) = self.failure(key) {
            return Err(error);
        }
        if !job.queued.swap(true, Ordering::AcqRel) {
            let _admission = self.0.admission.lock().unwrap_or_else(|e| e.into_inner());
            if self.0.closed.load(Ordering::Acquire) {
                return Err(MediaError::Unavailable);
            }
            let cache = self.clone();
            let job = job.clone();
            self.0.tasks.spawn(async move {
                let guard = Running::new(cache.clone(), job.clone());
                let work = async {
                    let _permit = tokio::select! { biased;
                        _ = cache.0.stop.cancelled() => return Err(MediaError::Unavailable),
                        _ = job.abandoned() => return Err(MediaError::Unavailable),
                        permit = cache.0.preview_slots.clone().acquire_owned() => permit.map_err(|_| MediaError::Unavailable)?,
                    };
                    match key.kind() {
                        MediaKind::Preview => cache.0.counters.renders.fetch_add(1, Ordering::Relaxed),
                        MediaKind::Cover => cache.0.counters.covers.fetch_add(1, Ordering::Relaxed),
                        _ => unreachable!(),
                    };
                    let asset = build(cache.clone(), job.clone()).await?;
                    cache.store(key, asset).await
                };
                guard.finish(work.await);
            });
        }
        job.wait(self.0.limits.request_timeout).await
    }
    pub(crate) fn preview_key(&self, document: crate::DocumentId) -> Option<MediaKey> {
        self.0.disk.preview_key(document)
    }
    pub(crate) fn failure(&self, key: MediaKey) -> Option<MediaError> {
        let mut failures = self.0.failures.lock().unwrap_or_else(|e| e.into_inner());
        failures.retain(|_, (until, _)| *until > Instant::now());
        failures.get(&key).map(|(until, error)| match error {
            MediaError::RetryAfter(_) => MediaError::RetryAfter(
                until.saturating_duration_since(Instant::now()).as_secs() + 1,
            ),
            error => *error,
        })
    }
    pub(crate) fn failed(&self, key: MediaKey, error: MediaError) {
        let seconds = match error {
            MediaError::NotFound | MediaError::InvalidMedia => 300,
            MediaError::RetryAfter(seconds) => seconds,
            _ => 2,
        };
        if let Some(until) = Instant::now().checked_add(std::time::Duration::from_secs(seconds)) {
            let mut failures = self.0.failures.lock().unwrap_or_else(|e| e.into_inner());
            failures.retain(|_, (until, _)| *until > Instant::now());
            if failures.len() >= 4096
                && let Some(old) = failures
                    .iter()
                    .min_by_key(|(_, entry)| entry.0)
                    .map(|(key, _)| *key)
            {
                failures.remove(&old);
            }
            failures.insert(key, (until, error));
        }
    }
    pub(crate) async fn store(&self, key: MediaKey, asset: MediaAsset) -> MediaResult {
        if asset.kind() != key.kind() {
            return Err(MediaError::InvalidMedia);
        }
        let disk = self.0.disk.clone();
        let saved = self.blocking(move || disk.write(key, &asset)).await?;
        self.remember(key, &saved);
        Ok(saved)
    }
    pub(crate) async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, MediaError> + Send + 'static,
    ) -> Result<T, MediaError> {
        // The blocking closure owns the permit: cancelling an HTTP caller must
        // not release disk capacity while its read/write is still running.
        let permit = self
            .0
            .io_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| MediaError::Unavailable)?;
        let task = {
            let _guard = self.0.admission.lock().unwrap_or_else(|e| e.into_inner());
            if self.0.closed.load(Ordering::Acquire) {
                return Err(MediaError::Unavailable);
            }
            self.0.tasks.spawn_blocking(move || {
                let _permit = permit;
                work()
            })
        };
        task.await.map_err(|_| MediaError::Unavailable)?
    }
    pub(crate) async fn maintain(&self) {
        let disk = self.0.disk.clone();
        if self.blocking(move || disk.maintain()).await.is_err() {
            tracing::debug!("media cache maintenance skipped");
        }
    }
    pub async fn shutdown(&self) {
        {
            let _guard = self.0.admission.lock().unwrap_or_else(|e| e.into_inner());
            self.0.closed.store(true, Ordering::Release);
            self.0.stop.cancel();
            self.0.queue.close();
            for job in self
                .0
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .filter_map(Weak::upgrade)
            {
                job.result.send_replace(Some(Err(MediaError::Unavailable)));
            }
            self.0.io_slots.close();
            self.0.tasks.close();
        }
        self.0.tasks.wait().await;
    }
    pub fn stats(&self) -> MediaStats {
        MediaStats {
            memory_hits: self.0.counters.hits.load(Ordering::Relaxed),
            disk_reads: self.0.counters.reads.load(Ordering::Relaxed),
            downloads: self.0.counters.downloads.load(Ordering::Relaxed),
            skipped_downloads: self.0.counters.skipped.load(Ordering::Relaxed),
            preview_generations: self.0.counters.renders.load(Ordering::Relaxed),
            cover_generations: self.0.counters.covers.load(Ordering::Relaxed),
            memory_bytes: self
                .0
                .memory
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .bytes,
            queued: self.0.queue.len(),
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_callers_do_not_detach_disk_work_from_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let (cache, downloads) = MediaCache::open(directory.path()).await.unwrap();
        drop(downloads);
        let (release, wait) = std::sync::mpsc::channel();
        let started = Arc::new(tokio::sync::Notify::new());
        let notified = started.clone();
        let reader = cache.clone();
        let work = tokio::spawn(async move {
            reader
                .blocking(move || {
                    notified.notify_one();
                    wait.recv().unwrap();
                    Ok(())
                })
                .await
        });
        started.notified().await;
        work.abort();
        let _ = work.await;
        assert_eq!(
            cache.0.io_slots.available_permits(),
            3,
            "Cancelled callers retain their disk permit until IO completes"
        );
        let stopping = cache.clone();
        let shutdown = tokio::spawn(async move {
            stopping.shutdown().await;
        });
        tokio::task::yield_now().await;
        assert!(!shutdown.is_finished());
        release.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), shutdown)
            .await
            .unwrap()
            .unwrap();
    }
}
