//! Process-owned task lifetime, panic reporting and graceful draining.
use anyhow::{Result, anyhow, bail};
use futures_util::FutureExt;
use std::{collections::BTreeSet, future::Future, panic::AssertUnwindSafe, sync::Mutex};
use tokio::{runtime::Handle, sync::watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TaskKind {
    MarketData,
    Valuation,
    Replication,
    #[cfg(feature = "bot")]
    Bot,
    #[cfg(feature = "bot")]
    MediaWarmup,
    Api,
}
#[derive(Debug, Clone, Copy)]
enum Failure {
    Panicked,
    UnexpectedExit,
    Failed,
}
#[derive(Clone, Copy)]
enum Lifetime {
    Service,
    #[cfg(feature = "bot")]
    Job,
}
#[derive(Debug, Clone, Copy)]
struct TaskFailure {
    task: TaskKind,
    failure: Failure,
}

pub(crate) struct TaskSupervisor {
    tracker: TaskTracker,
    stop: CancellationToken,
    runtime: Handle,
    admission: Mutex<BTreeSet<TaskKind>>,
    failure: watch::Sender<Option<TaskFailure>>,
}
fn stopped() -> anyhow::Error {
    anyhow!("Application is stopping")
}
impl TaskSupervisor {
    pub fn new() -> Self {
        let (failure, _) = watch::channel(None);
        Self {
            tracker: TaskTracker::new(),
            stop: CancellationToken::new(),
            runtime: Handle::current(),
            admission: Mutex::default(),
            failure,
        }
    }
    pub fn token(&self) -> CancellationToken {
        self.stop.child_token()
    }
    pub fn is_stopping(&self) -> bool {
        self.stop.is_cancelled()
    }
    pub async fn cancelled(&self) {
        self.stop.cancelled().await;
    }
    async fn monitor(
        task: TaskKind,
        work: impl Future<Output = bool>,
        stop: CancellationToken,
        failure: watch::Sender<Option<TaskFailure>>,
        critical: bool,
        lifetime: Lifetime,
    ) {
        let result = AssertUnwindSafe(work).catch_unwind().await;
        let reason = if result.is_err() {
            Some(Failure::Panicked)
        } else if matches!(result, Ok(false)) {
            Some(Failure::Failed)
        } else if matches!(lifetime, Lifetime::Service) && !stop.is_cancelled() {
            Some(Failure::UnexpectedExit)
        } else {
            None
        };
        if let Some(reason) = reason {
            tracing::error!(?task, ?reason, "background task failed");
            if critical {
                failure.send_replace(Some(TaskFailure {
                    task,
                    failure: reason,
                }));
                stop.cancel();
            }
        }
    }
    pub fn spawn_critical(
        &self,
        task: TaskKind,
        work: impl Future<Output = std::result::Result<(), ()>> + Send + 'static,
    ) -> Result<()> {
        self.spawn_checked(task, async move { work.await.is_ok() })
    }
    fn spawn_checked(
        &self,
        task: TaskKind,
        work: impl Future<Output = bool> + Send + 'static,
    ) -> Result<()> {
        self.spawn_work(task, work, true, Lifetime::Service)
    }
    #[cfg(feature = "bot")]
    pub fn spawn_optional(
        &self,
        task: TaskKind,
        work: impl Future<Output = std::result::Result<(), ()>> + Send + 'static,
    ) -> Result<()> {
        self.spawn_work(
            task,
            async move { work.await.is_ok() },
            false,
            Lifetime::Service,
        )
    }
    #[cfg(feature = "bot")]
    pub fn spawn_job(
        &self,
        task: TaskKind,
        work: impl Future<Output = std::result::Result<(), ()>> + Send + 'static,
    ) -> Result<()> {
        self.spawn_work(
            task,
            async move { work.await.is_ok() },
            false,
            Lifetime::Job,
        )
    }
    fn spawn_work(
        &self,
        task: TaskKind,
        work: impl Future<Output = bool> + Send + 'static,
        critical: bool,
        lifetime: Lifetime,
    ) -> Result<()> {
        let mut admission = self.admission.lock().map_err(|_| stopped())?;
        if self.is_stopping() {
            return Err(stopped());
        }
        if !admission.insert(task) {
            bail!("{task:?} worker was already started");
        }
        self.tracker.spawn_on(
            Self::monitor(
                task,
                work,
                self.stop.clone(),
                self.failure.clone(),
                critical,
                lifetime,
            ),
            &self.runtime,
        );
        Ok(())
    }
    pub fn begin_shutdown(&self) -> Result<()> {
        {
            let _admission = self.admission.lock().map_err(|_| stopped())?;
            self.stop.cancel();
            self.tracker.close();
        }
        Ok(())
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.begin_shutdown()?;
        self.tracker.wait().await;
        if let Some(TaskFailure { task, failure }) = *self.failure.borrow() {
            bail!("{task:?} task failed: {failure:?}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};

    #[cfg(feature = "bot")]
    #[tokio::test]
    async fn finite_job_completion_is_expected_and_failures_keep_their_policy() {
        for success in [true, false] {
            let supervisor = TaskSupervisor::new();
            supervisor
                .spawn_work(
                    TaskKind::MediaWarmup,
                    async move { success },
                    true,
                    Lifetime::Job,
                )
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                while !supervisor.tracker.is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(supervisor.is_stopping(), !success);
            assert_eq!(supervisor.shutdown().await.is_ok(), success);
        }
    }
    #[cfg(feature = "bot")]
    #[tokio::test]
    async fn finite_jobs_remain_tracked_until_drained_and_cannot_start_twice() {
        let supervisor = Arc::new(TaskSupervisor::new());
        let (release, finish) = tokio::sync::oneshot::channel();
        supervisor
            .spawn_job(TaskKind::MediaWarmup, async move {
                let _ = finish.await;
                Ok(())
            })
            .unwrap();
        assert!(
            supervisor
                .spawn_job(TaskKind::MediaWarmup, async { Ok(()) })
                .is_err()
        );
        let target = supervisor.clone();
        let shutdown = tokio::spawn(async move { target.shutdown().await });
        supervisor.cancelled().await;
        assert!(!shutdown.is_finished());
        release.send(()).unwrap();
        shutdown.await.unwrap().unwrap();
        assert!(supervisor.tracker.is_empty());
    }
    #[tokio::test]
    async fn shutdown_drains_workers_and_rejects_new_tasks() {
        let supervisor = Arc::new(TaskSupervisor::new());
        let (started, running) = tokio::sync::oneshot::channel();
        let (release, finish) = tokio::sync::oneshot::channel();
        supervisor
            .spawn_critical(TaskKind::MarketData, async move {
                started.send(()).unwrap();
                let _ = finish.await;
                Ok(())
            })
            .unwrap();
        running.await.unwrap();
        let target = supervisor.clone();
        let shutdown = tokio::spawn(async move { target.shutdown().await });
        supervisor.cancelled().await;
        assert!(!shutdown.is_finished());
        assert!(
            supervisor
                .spawn_checked(TaskKind::Api, async { true })
                .is_err()
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), shutdown)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(supervisor.tracker.is_empty());
    }

    #[tokio::test]
    async fn critical_exit_stops_the_process_and_reports_failure() {
        for (kind, panics) in [
            (TaskKind::MarketData, false),
            (TaskKind::MarketData, true),
            (TaskKind::Replication, false),
            (TaskKind::Replication, true),
            #[cfg(feature = "bot")]
            (TaskKind::Bot, false),
            #[cfg(feature = "bot")]
            (TaskKind::Bot, true),
            (TaskKind::Api, false),
            (TaskKind::Api, true),
        ] {
            let supervisor = TaskSupervisor::new();
            supervisor
                .spawn_checked(kind, async move {
                    assert!(!panics, "simulated critical worker failure");
                    true
                })
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), supervisor.cancelled())
                .await
                .unwrap();
            assert!(supervisor.shutdown().await.is_err());
            assert!(supervisor.tracker.is_empty());
        }
    }

    #[tokio::test]
    async fn singleton_workers_cannot_be_started_twice() {
        let supervisor = TaskSupervisor::new();
        for kind in [
            TaskKind::MarketData,
            TaskKind::Replication,
            #[cfg(feature = "bot")]
            TaskKind::Bot,
            TaskKind::Api,
        ] {
            let stop = supervisor.token();
            supervisor
                .spawn_checked(kind, async move {
                    stop.cancelled().await;
                    true
                })
                .unwrap();
            assert!(supervisor.spawn_checked(kind, async { true }).is_err());
        }
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn explicit_worker_failure_is_reported_even_during_shutdown() {
        let supervisor = TaskSupervisor::new();
        let (release, finish) = tokio::sync::oneshot::channel();
        supervisor
            .spawn_critical(TaskKind::MarketData, async move {
                let _ = finish.await;
                Err(())
            })
            .unwrap();
        supervisor.begin_shutdown().unwrap();
        release.send(()).unwrap();
        assert!(supervisor.shutdown().await.is_err());
    }
    #[cfg(feature = "bot")]
    #[tokio::test]
    async fn optional_bot_failures_and_panics_leave_the_api_running() {
        for panics in [false, true] {
            let supervisor = TaskSupervisor::new();
            let stop = supervisor.token();
            supervisor
                .spawn_critical(TaskKind::Api, async move {
                    stop.cancelled().await;
                    Ok(())
                })
                .unwrap();
            supervisor
                .spawn_optional(TaskKind::Bot, async move {
                    assert!(!panics, "simulated bot failure");
                    Err(())
                })
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                while supervisor.tracker.len() > 1 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(!supervisor.is_stopping());
            assert!(
                supervisor
                    .spawn_optional(TaskKind::Bot, async { Ok(()) })
                    .is_err()
            );
            supervisor.shutdown().await.unwrap();
        }
    }
}
