//! API-owned connection tasks and restricted access for HTTP workers.
use crate::error::{ApiError, Result};
use futures_util::FutureExt;
use std::{
    future::Future,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone)]
pub(crate) struct ConnectionSupervisor {
    tasks: Arc<ConnectionTasks>,
}
struct ConnectionTasks {
    tracker: TaskTracker,
    stop: CancellationToken,
    admission: Mutex<()>,
    slots: Arc<tokio::sync::Semaphore>,
}
impl ConnectionSupervisor {
    #[cfg(test)]
    pub fn new(stop: CancellationToken) -> Self {
        Self::with_limit(stop, crate::config::WebSocketLimits::default().connections)
    }
    pub fn with_limit(stop: CancellationToken, limit: usize) -> Self {
        Self {
            tasks: Arc::new(ConnectionTasks {
                tracker: TaskTracker::new(),
                stop,
                admission: Mutex::new(()),
                slots: Arc::new(tokio::sync::Semaphore::new(limit)),
            }),
        }
    }
    /// Runs a connection with isolated cancellation, on its HTTP worker.
    pub fn spawn_local<F>(&self, work: impl FnOnce(CancellationToken) -> F + 'static) -> Result<()>
    where
        F: Future<Output = ()> + 'static,
    {
        let _admission = self
            .tasks
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.tasks.stop.is_cancelled() {
            return Err(ApiError::from(
                market_actors::error::DataError::unavailable("API is stopping"),
            ));
        }
        let permit = self
            .tasks
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::capacity("WebSocket connection limit reached"))?;
        let stop = self.tasks.stop.child_token();
        self.tasks.tracker.spawn_local(async move {
            let _permit = permit;
            let _cancel_on_exit = stop.clone().drop_guard();
            if AssertUnwindSafe(async move { work(stop).await })
                .catch_unwind()
                .await
                .is_err()
            {
                tracing::warn!("WebSocket connection task panicked");
            }
        });
        Ok(())
    }
    pub fn begin_shutdown(&self) {
        let _admission = self
            .tasks
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.tasks.stop.cancel();
        self.tasks.tracker.close();
    }
    pub async fn shutdown(&self) {
        self.begin_shutdown();
        self.tasks.tracker.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::oneshot;

    #[actix_web::test]
    async fn connection_cancellation_is_isolated_and_shutdown_rejects_and_drains_work() {
        let stop = CancellationToken::new();
        let connections = ConnectionSupervisor::new(stop.clone());
        let (first, running) = oneshot::channel();
        connections
            .spawn_local(move |stop| async move {
                first.send(stop.clone()).unwrap();
                stop.cancelled().await;
            })
            .unwrap();
        let first = running.await.unwrap();

        let (second, running) = oneshot::channel();
        let (drain, finish) = oneshot::channel();
        connections
            .spawn_local(move |stop| async move {
                second.send(stop.clone()).unwrap();
                stop.cancelled().await;
                // Model connection cleanup that must finish before runtime shutdown returns.
                let _ = finish.await;
            })
            .unwrap();
        let second = running.await.unwrap();
        first.cancel();
        assert!(!second.is_cancelled());
        assert!(!stop.is_cancelled());

        connections.begin_shutdown();
        assert!(second.is_cancelled());
        assert!(
            connections
                .spawn_local(|_| async { panic!("work admitted after shutdown") })
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), connections.shutdown())
                .await
                .is_err()
        );
        drain.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), connections.shutdown())
            .await
            .unwrap();
    }

    #[actix_web::test]
    async fn connection_panic_cancels_its_children_and_does_not_stop_other_connections() {
        let stop = CancellationToken::new();
        let connections = ConnectionSupervisor::new(stop.clone());
        let (started, running) = oneshot::channel();
        connections
            .spawn_local(move |stop| async move {
                started.send(stop.child_token()).unwrap();
                panic!("simulated connection failure");
            })
            .unwrap();
        let child = running.await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), child.cancelled())
            .await
            .unwrap();
        assert!(!stop.is_cancelled());
        let (started, running) = oneshot::channel();
        connections
            .spawn_local(move |_| async move { started.send(()).unwrap() })
            .unwrap();
        running.await.unwrap();
        connections.shutdown().await;
    }
}
