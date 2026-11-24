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
