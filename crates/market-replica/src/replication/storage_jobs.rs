//! All staging, journal, replay and cleanup writes share one serial storage executor.
use std::sync::Arc;
use tokio_util::task::{TaskTracker, task_tracker::TaskTrackerToken};
use tonic::Status;
#[derive(Clone)]
pub(crate) struct StorageJobs {
    tracker: TaskTracker,
    pool: Arc<crate::blocking::BlockingPool>,
}
impl StorageJobs {
    pub fn new() -> Result<Self, Status> {
        Ok(Self {
            tracker: TaskTracker::new(),
            pool: Arc::new(crate::blocking::BlockingPool::new("market-storage", 1)?),
        })
    }
    pub async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, Status> + Send + 'static,
    ) -> Result<T, Status> {
        // Track returned ownership too: abandoned output must release its build
        // lease before recovery can start another snapshot.
        let token = self.tracker.token();
        let (result, _token) = self
            .pool
            .spawn(move || (work(), token))
            .await
            .map_err(|_| Status::unavailable("Replica storage worker stopped"))?;
        result
    }
    pub async fn drain(&self) {
        self.tracker.close();
        self.tracker.wait().await;
    }
    pub fn reopen(&self) {
        self.tracker.reopen();
    }
    #[cfg(test)]
    pub fn close(&self) {
        self.tracker.close();
    }
    #[cfg(test)]
    pub async fn wait(&self) {
        self.tracker.wait().await;
    }
    pub async fn shutdown(&self) {
        self.drain().await;
        self.pool.shutdown();
    }
}

/// Returned blocking results can wait in async prefetch buffers. Abandoning a
/// large builder/view must release its indexes, database and lease off-thread,
/// and recovery must drain that cleanup before starting another build.
pub(crate) struct StorageValue<T: Send + 'static> {
    value: Option<T>,
    token: Option<TaskTrackerToken>,
    jobs: StorageJobs,
}
impl<T: Send + 'static> StorageValue<T> {
    pub(crate) fn new(value: T, jobs: &StorageJobs) -> Self {
        Self {
            value: Some(value),
            token: Some(jobs.tracker.token()),
            jobs: jobs.clone(),
        }
    }
    pub(super) fn get(&mut self) -> &mut T {
        self.value.as_mut().expect("storage value is present")
    }
    /// Transfer only when no await can abandon the unguarded result, or inside a storage job.
    pub(crate) fn take(mut self) -> T {
        self.value.take().expect("storage value is present")
    }
}
impl<T: Send + 'static> Drop for StorageValue<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            let token = self.token.take().expect("storage value is tracked");
            // Keep the executor alive until queued destruction actually runs.
            // Otherwise dropping its last owner could cancel this cleanup job
            // and destroy the database/result on the caller's async thread.
            let cleanup = self.jobs.clone();
            self.jobs.pool.spawn(move || {
                drop(value);
                drop(token);
                drop(cleanup);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn queued_cleanup_survives_dropping_its_last_async_owner() {
        struct Notify(Option<tokio::sync::oneshot::Sender<String>>);
        impl Drop for Notify {
            fn drop(&mut self) {
                let _ = self
                    .0
                    .take()
                    .unwrap()
                    .send(std::thread::current().name().unwrap().to_string());
            }
        }
        let jobs = StorageJobs::new().unwrap();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let blocked = jobs.pool.spawn(move || {
            entered.send(()).unwrap();
            wait.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        });
        ready.await.unwrap();
        let (send, receive) = tokio::sync::oneshot::channel();
        drop(StorageValue::new(Notify(Some(send)), &jobs));
        drop(jobs);
        release.send(()).unwrap();
        blocked.await.unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), receive)
                .await
                .unwrap()
                .unwrap(),
            "market-storage"
        );
    }
    #[tokio::test]
    async fn abandoned_output_is_destroyed_on_the_storage_worker_before_drain_completes() {
        struct Probe(Option<tokio::sync::oneshot::Sender<String>>);
        impl Drop for Probe {
            fn drop(&mut self) {
                let _ = self
                    .0
                    .take()
                    .unwrap()
                    .send(std::thread::current().name().unwrap().to_string());
            }
        }
        let jobs = StorageJobs::new().unwrap();
        let cleanup = jobs.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        let output = jobs
            .run(move || Ok(StorageValue::new(Probe(Some(send)), &cleanup)))
            .await
            .unwrap();
        drop(output);
        jobs.shutdown().await;
        assert_eq!(receive.await.unwrap(), "market-storage");
    }
    #[tokio::test]
    async fn storage_is_serial_and_cleanup_uses_the_same_worker() {
        let jobs = StorageJobs::new().unwrap();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, proceed) = std::sync::mpsc::channel();
        let first_jobs = jobs.clone();
        let first = tokio::spawn(async move {
            first_jobs
                .run(move || {
                    entered.send(()).unwrap();
                    proceed.recv().unwrap();
                    Ok(())
                })
                .await
                .unwrap();
        });
        started.await.unwrap();
        let mut second =
            Box::pin(jobs.run(|| Ok(std::thread::current().name().unwrap().to_string())));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut second)
                .await
                .is_err()
        );
        release.send(()).unwrap();
        first.await.unwrap();
        assert_eq!(second.await.unwrap(), "market-storage");
        jobs.shutdown().await;
    }
}
