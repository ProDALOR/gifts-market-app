//! Assemble a read capability and one ordinary synchronization worker.
use crate::{ReplicaReader, ReplicaWriter, ReplicationWorker};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tonic::Status;

pub struct ReplicaRuntime {
    pub reader: ReplicaReader,
    pub replication: ReplicationWorker,
}
impl ReplicaRuntime {
    pub async fn open(directory: &Path, source: &str) -> Result<Self, Status> {
        Self::open_with_cancellation(
            directory,
            source,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
    }
    /// Cancellation stops between database operations/rows. An in-progress redb
    /// open or commit finishes before locks and partial state can be released.
    pub async fn open_with_cancellation(
        directory: &Path,
        source: &str,
        stop: tokio_util::sync::CancellationToken,
    ) -> Result<Self, Status> {
        Self::open_checked(directory, source, stop, || Ok(())).await
    }
    async fn open_checked(
        directory: &Path,
        source: &str,
        stop: tokio_util::sync::CancellationToken,
        check: impl Fn() -> Result<(), Status> + Send + Sync + 'static,
    ) -> Result<Self, Status> {
        use crate::replication::storage_jobs::{StorageJobs, StorageValue};
        let stop = stop.child_token();
        let _cancel = stop.clone().drop_guard();
        let check = Arc::new(move || {
            check()?;
            if stop.is_cancelled() {
                Err(Status::cancelled("Replica startup cancelled"))
            } else {
                Ok(())
            }
        });
        check()?;
        let directory = directory.to_owned();
        let source = source.to_owned();
        let jobs = StorageJobs::new()?;
        let cleanup = jobs.clone();
        let worker_check = check.clone();
        let result = jobs
            .run(move || {
                let writer =
                    ReplicaWriter::open_checked(&directory, &source, worker_check.as_ref())?;
                worker_check()?;
                let runtime = Self {
                    reader: writer.reader(),
                    replication: ReplicationWorker {
                        writer: Arc::new(Mutex::new(writer)),
                    },
                };
                // If the async caller disappeared, destroy the result on storage
                // workers, including its database and directory lock.
                Ok(StorageValue::new(runtime, &cleanup))
            })
            .await?;
        if let Err(error) = check() {
            drop(result);
            jobs.shutdown().await;
            return Err(error);
        }
        // No await after ownership leaves the cleanup guard.
        Ok(result.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::w;
    use crate::{
        storage::Store,
        test_support::{entity, order, upsert},
    };
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn pre_cancelled_startup_does_not_create_a_directory() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unused");
        let stop = CancellationToken::new();
        stop.cancel();
        let error = ReplicaRuntime::open_with_cancellation(&path, "test", stop)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code(), tonic::Code::Cancelled);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn cancelled_or_abandoned_startup_releases_the_lock_and_preserves_checkpoint() {
        for abandoned in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let store = Store::open(&directory.path().join("replica.redb"), "test").unwrap();
            store
                .apply(
                    &[upsert(entity(w::entity::Entity::Order(order(
                        "active", "2", None,
                    ))))],
                    Some("saved"),
                )
                .unwrap();
            drop(store);
            let stop = CancellationToken::new();
            let worker_stop = stop.clone();
            let path = directory.path().to_owned();
            let (entered, ready) = tokio::sync::oneshot::channel();
            let entered = Mutex::new(Some(entered));
            let (release, wait) = std::sync::mpsc::channel();
            let wait = Mutex::new(wait);
            let gated = AtomicBool::new(false);
            let lock_path = directory.path().join("replica.lock");
            let mut task = tokio::spawn(async move {
                ReplicaRuntime::open_checked(&path, "test", worker_stop, move || {
                    // Gate at a real resource boundary, independently of how
                    // many cancellation checks the implementation performs.
                    if !gated.load(Ordering::SeqCst)
                        && let Ok(lock) = std::fs::File::options()
                            .read(true)
                            .write(true)
                            .open(&lock_path)
                        && matches!(lock.try_lock(), Err(std::fs::TryLockError::WouldBlock))
                        && !gated.swap(true, Ordering::SeqCst)
                    {
                        entered.lock().unwrap().take().unwrap().send(()).unwrap();
                        wait.lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                    }
                    Ok(())
                })
                .await
            });
            tokio::time::timeout(Duration::from_secs(3), ready)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                ReplicaWriter::open(directory.path(), "test")
                    .err()
                    .unwrap()
                    .code(),
                tonic::Code::FailedPrecondition
            );
            if abandoned {
                task.abort();
                assert!((&mut task).await.err().unwrap().is_cancelled());
            } else {
                stop.cancel();
            }
            release.send(()).unwrap();
            if !abandoned {
                assert_eq!(
                    (&mut task).await.unwrap().err().unwrap().code(),
                    tonic::Code::Cancelled
                );
            }
            let path = directory.path().to_owned();
            let reopened = tokio::time::timeout(Duration::from_secs(3), async move {
                loop {
                    let path = path.clone();
                    let opened =
                        tokio::task::spawn_blocking(move || ReplicaWriter::open(&path, "test"))
                            .await
                            .unwrap();
                    match opened {
                        Ok(writer) => break writer,
                        Err(error) if error.code() == tonic::Code::FailedPrecondition => {
                            tokio::time::sleep(Duration::from_millis(5)).await
                        }
                        Err(error) => panic!("reopen failed: {error}"),
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(reopened.cursor().unwrap(), "saved");
            let key = crate::state::Key::order("venue".into(), "active".into());
            assert_eq!(
                crate::state::order(
                    reopened
                        .reader()
                        .view()
                        .unwrap()
                        .state
                        .entity(&key)
                        .unwrap()
                )
                .unwrap()
                .price,
                "2"
            );
        }
    }
}
