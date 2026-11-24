//! A supervised async worker owns its runtime, I/O, shutdown and thread lifetime.
#[cfg(feature = "bot")]
use std::future::Future;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "bot")]
pub(super) async fn run<F, T>(
    name: &'static str,
    stop: CancellationToken,
    factory: F,
) -> Result<(), ()>
where
    F: FnOnce(CancellationToken) -> T + Send + 'static,
    T: Future<Output = Result<(), ()>> + 'static,
{
    let local = stop.child_token();
    let _cancel = local.clone().drop_guard();
    let (send, receive) = tokio::sync::oneshot::channel();
    let thread = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(2)
                    .thread_name(name)
                    .build()
                    .map_err(|error| {
                        tracing::error!(%error, name, "worker runtime failed");
                    })?;
                runtime.block_on(async move {
                    let lag_stop = local.child_token();
                    let lag = tokio::spawn(monitor(name, lag_stop.clone()));
                    let result = factory(local).await;
                    lag_stop.cancel();
                    let _ = lag.await;
                    result
                })
            }))
            .unwrap_or_else(|_| {
                tracing::error!(name, "worker thread panicked");
                Err(())
            });
            // Notify only after runtime teardown, including blocking I/O cleanup.
            let _ = send.send(result);
        })
        .map_err(|error| {
            tracing::error!(%error, name, "worker thread failed to start");
        })?;
    let result = receive.await.unwrap_or(Err(()));
    // Completion is sent at thread exit; join off the parent event loop.
    tokio::task::spawn_blocking(move || thread.join())
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
    result
}

/// Drain service-owned work before forwarding a panic to the thread supervisor.
#[cfg(feature = "bot")]
pub(super) async fn with_cleanup<T>(
    work: impl Future<Output = T>,
    cleanup: impl Future<Output = ()>,
) -> T {
    use futures_util::FutureExt;
    let result = std::panic::AssertUnwindSafe(work).catch_unwind().await;
    cleanup.await;
    match result {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

pub(super) async fn monitor(name: &'static str, stop: CancellationToken) -> Result<(), ()> {
    let period = Duration::from_secs(1);
    let mut next = tokio::time::Instant::now() + period;
    loop {
        tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            _ = tokio::time::sleep_until(next) => {
                let delay = tokio::time::Instant::now().saturating_duration_since(next);
                if delay > Duration::from_millis(100) {
                    tracing::warn!(worker=name, delay_ms=delay.as_millis(), "worker event loop delayed");
                }
                next = tokio::time::Instant::now() + period;
            }
        }
    }
}

#[cfg(all(test, feature = "bot"))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn worker_has_its_own_thread_and_drains_on_shutdown() {
        let stop = CancellationToken::new();
        let child = stop.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run("isolated-test", child, |stop| async move {
            send.send(std::thread::current().name().unwrap().to_string())
                .unwrap();
            stop.cancelled().await;
            Ok(())
        }));
        assert_eq!(receive.await.unwrap(), "isolated-test");
        stop.cancel();
        task.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn panicking_service_drains_cleanup_before_thread_completion() {
        let cleaned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = cleaned.clone();
        let result = run(
            "cleanup-test",
            CancellationToken::new(),
            move |_| async move {
                with_cleanup(
                    async {
                        panic!("service panic");
                    },
                    async {
                        flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    },
                )
                .await
            },
        )
        .await;
        assert!(result.is_err());
        assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
    }
    #[tokio::test]
    async fn thread_panics_are_reported() {
        assert!(
            run("panic-test", CancellationToken::new(), |_| async {
                panic!("worker test");
            })
            .await
            .is_err()
        );
    }
}
