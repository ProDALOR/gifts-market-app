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
        }
    }
    pub fn demand(self: &Arc<Self>, priority: MediaPriority) -> Demand {
        self.waiters.fetch_add(1, Ordering::AcqRel);
        self.priority.fetch_max(priority as u8, Ordering::AcqRel);
        Demand(self.clone())
    }
}
pub(crate) struct Demand(Arc<Job>);
impl Drop for Demand {
    fn drop(&mut self) {
        self.0.waiters.fetch_sub(1, Ordering::AcqRel);
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
                let guard = Running {
                    cache: self.cache.clone(),
                    job,
                    complete: false,
                };
                let future = fetch(guard.job.key);
                let timeout = limits.download_timeout;
                self.cache
                    .0
                    .counters
                    .downloads
                    .fetch_add(1, Ordering::Relaxed);
                active.push(async move {
                    let mut guard = guard;
                    let result = tokio::time::timeout(timeout, future)
                        .await
                        .unwrap_or(Err(MediaError::Unavailable));
                    let result = match result {
                        Ok(asset) => guard.cache.store(guard.job.key, asset).await,
                        Err(error) => Err(error),
                    };
                    if let Err(error) = result {
                        guard.cache.failed(guard.job.key, error);
                        tracing::debug!(
                            document_id = guard.job.key.document().get(),
                            ?error,
                            "media download failed"
                        );
                    }
                    guard.job.result.send_replace(Some(result));
                    guard.cache.retire(&guard.job);
                    guard.complete = true;
                });
                next = tokio::time::Instant::now() + limits.start_interval;
            }
            if queue.closed.load(Ordering::Acquire) && active.is_empty() {
                return;
            }
            let paced = queue.len() > 0 && active.len() < limits.downloads;
            tokio::select! {
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
struct Running {
    cache: MediaCache,
    job: Arc<Job>,
    complete: bool,
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
