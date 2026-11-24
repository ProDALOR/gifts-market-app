//! Dedicated bounded blocking execution. Dropping an owner never blocks an async worker.
use std::{sync::Mutex, time::Instant};
use tokio::{
    runtime::{Builder, Runtime},
    task::JoinHandle,
};
use tonic::Status;

pub(crate) struct BlockingPool {
    runtime: Mutex<Option<Runtime>>,
    handle: tokio::runtime::Handle,
    name: &'static str,
}
impl BlockingPool {
    pub fn new(name: &'static str, workers: usize) -> Result<Self, Status> {
        let runtime = Builder::new_current_thread()
            .thread_name(name)
            .max_blocking_threads(workers.max(1))
            .build()
            .map_err(|_| Status::unavailable("Cannot start market execution pool"))?;
        let handle = runtime.handle().clone();
        Ok(Self {
            runtime: Mutex::new(Some(runtime)),
            handle,
            name,
        })
    }
    pub fn shutdown(&self) {
        if let Some(runtime) = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            runtime.shutdown_background();
        }
    }
    pub fn spawn<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> JoinHandle<T> {
        let queued = Instant::now();
        let name = self.name;
        self.handle.spawn_blocking(move || {
            let wait = queued.elapsed();
            let start = Instant::now();
            let result = work();
            tracing::debug!(
                pool = name,
                queue_ms = wait.as_millis(),
                execution_ms = start.elapsed().as_millis(),
                "market blocking job completed"
            );
            result
        })
    }
}
impl Drop for BlockingPool {
    fn drop(&mut self) {
        if let Some(runtime) = self
            .runtime
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            runtime.shutdown_background();
        }
    }
}
