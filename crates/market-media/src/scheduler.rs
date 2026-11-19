use crate::{MediaCache, MediaError, MediaKey, MediaResult};
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::VecDeque,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Notify, OnceCell, watch};
#[derive(Debug, Clone, Copy)]
pub enum MediaPriority {
    Prefetch,
    Interactive,
}
pub(crate) struct Job {
    pub key: MediaKey,
    pub disk: OnceCell<Result<Option<crate::MediaAsset>, MediaError>>,
    pub result: watch::Sender<Option<MediaResult>>,
    pub queued: AtomicBool,
    waiters: AtomicUsize,
    priority: AtomicU8,
    abandoned: Notify,
}
impl Job {
    pub fn new(key: MediaKey) -> Self {
        Self {
            key,
            disk: OnceCell::new(),
            result: watch::channel(None).0,
            queued: AtomicBool::new(false),
            waiters: AtomicUsize::new(0),
            priority: AtomicU8::new(0),
            abandoned: Notify::new(),
        }
    }
    pub fn demand(self: &Arc<Self>, priority: MediaPriority) -> Demand {
        self.waiters.fetch_add(1, Ordering::AcqRel);
        self.priority.fetch_max(priority as u8, Ordering::AcqRel);
        Demand(self.clone())
    }
    pub(crate) async fn abandoned(&self) {
        loop {
            let changed = self.abandoned.notified();
            if self.waiters.load(Ordering::Acquire) == 0 {
                return;
            }
            changed.await;
        }
    }
    pub(crate) fn has_demand(&self) -> bool {
        self.waiters.load(Ordering::Acquire) > 0
    }
    pub(crate) async fn wait(&self, timeout: std::time::Duration) -> MediaResult {
        let mut updates = self.result.subscribe();
        tokio::time::timeout(timeout, async {
            loop {
                if let Some(result) = updates.borrow_and_update().clone() {
                    return result;
                }
                updates
                    .changed()
                    .await
                    .map_err(|_| MediaError::Unavailable)?;
            }
        })
        .await
        .unwrap_or(Err(MediaError::Unavailable))
    }
}
pub(crate) struct Demand(Arc<Job>);
impl Drop for Demand {
    fn drop(&mut self) {
        if self.0.waiters.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.abandoned.notify_waiters();
        }
    }
}
#[derive(Default)]
pub(crate) struct Queue {
    jobs: Mutex<VecDeque<Arc<Job>>>,
    closed: AtomicBool,
    pub changed: Notify,
}
impl Queue {
    pub fn push(&self, job: Arc<Job>, limit: usize) -> bool {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if self.closed.load(Ordering::Acquire) || jobs.len() >= limit {
            return false;
        }
        jobs.push_back(job);
        self.changed.notify_one();
        true
    }
    fn pop(&self) -> Option<Arc<Job>> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let index = jobs
            .iter()
            .position(|job| {
                job.priority.load(Ordering::Acquire) > 0 && job.waiters.load(Ordering::Acquire) > 0
            })
            .unwrap_or(0);
        jobs.remove(index)
    }
    pub fn len(&self) -> usize {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        for job in self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            job.result.send_replace(Some(Err(MediaError::Unavailable)));
        }
        self.changed.notify_one();
    }
}
pub struct MediaDownloader {
    cache: MediaCache,
}
impl MediaDownloader {
    pub(crate) fn new(cache: MediaCache) -> Self {
        Self { cache }
    }
    /// Bounded parallel downloads, shared pacing, foreground priority and abandoned-job removal.
    /// Cancelling the transport publishes a retryable result; queued work survives reconnects.
    pub async fn run<F, Fut>(&mut self, fetch: F)
    where
        F: Fn(MediaKey) -> Fut,
        Fut: Future<Output = MediaResult>,
    {
        let queue = self.cache.0.queue.clone();
        let limits = self.cache.0.limits.clone();
        let mut active = FuturesUnordered::new();
        let mut next = tokio::time::Instant::now();
        let mut maintenance_tick = tokio::time::interval_at(
            next + std::time::Duration::from_secs(60),
            std::time::Duration::from_secs(60),
        );
        maintenance_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let maintenance = async {
            loop {
                maintenance_tick.tick().await;
                self.cache.maintain().await;
            }
        };
        tokio::pin!(maintenance);
        loop {
            while active.len() < limits.downloads && tokio::time::Instant::now() >= next {
                let Some(job) = queue.pop() else {
                    break;
                };
                if job.waiters.load(Ordering::Acquire) == 0 {
                    self.cache
                        .0
                        .counters
                        .skipped
                        .fetch_add(1, Ordering::Relaxed);
                    job.result.send_replace(Some(Err(MediaError::Unavailable)));
                    self.cache.retire(&job);
                    continue;
                }
                let guard = Running::new(self.cache.clone(), job);
                let future = fetch(guard.job.key);
                let timeout = limits.download_timeout;
                self.cache
                    .0
                    .counters
                    .downloads
                    .fetch_add(1, Ordering::Relaxed);
                active.push(async move {
                    let result = tokio::time::timeout(timeout, future)
                        .await
                        .unwrap_or(Err(MediaError::Unavailable));
                    let result = match result {
                        Ok(asset) => guard.cache.store(guard.job.key, asset).await,
                        Err(error) => Err(error),
                    };
                    guard.finish(result);
                });
                next = tokio::time::Instant::now() + limits.start_interval;
            }
            if queue.closed.load(Ordering::Acquire) && active.is_empty() {
                return;
            }
            let paced = queue.len() > 0 && active.len() < limits.downloads;
            tokio::select! {
                _=&mut maintenance=>{},
                _=active.next(),if !active.is_empty()=>{},
                _=queue.changed.notified()=>{},
                _=tokio::time::sleep_until(next),if paced=>{},
            }
        }
    }
}
impl Drop for MediaDownloader {
    fn drop(&mut self) {
        self.cache.0.queue.close();
    }
}
pub(crate) struct Running {
    pub(crate) cache: MediaCache,
    pub(crate) job: Arc<Job>,
    complete: bool,
}
impl Running {
    pub(crate) fn new(cache: MediaCache, job: Arc<Job>) -> Self {
        Self {
            cache,
            job,
            complete: false,
        }
    }
    pub(crate) fn finish(mut self, result: MediaResult) {
        if let Err(error) = result
            && self.job.has_demand()
            && !self.cache.0.stop.is_cancelled()
        {
            self.cache.failed(self.job.key, error);
            tracing::debug!(
                document_id = self.job.key.document().get(),
                ?error,
                "media job failed"
            );
        }
        self.job.result.send_replace(Some(result));
        self.cache.retire(&self.job);
        self.complete = true;
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        if !self.complete {
            self.job
                .result
                .send_replace(Some(Err(MediaError::Unavailable)));
            self.cache.retire(&self.job);
        }
    }
}
