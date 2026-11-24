//! Scoped startup diagnostics: slow work is visible without a detached ticker task.
use std::{future::Future, time::Duration};
use tokio::time::{Instant, MissedTickBehavior, interval_at};

pub async fn phase<T, E>(
    phase: &'static str,
    stop: &tokio_util::sync::CancellationToken,
    work: impl Future<Output = Result<T, E>>,
) -> anyhow::Result<T>
where
    E: Into<anyhow::Error>,
{
    anyhow::ensure!(!stop.is_cancelled(), "Application is stopping");
    let started = Instant::now();
    tracing::info!(phase, "startup phase started");
    let period = Duration::from_secs(5);
    let mut progress = interval_at(started + period, period);
    progress.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tokio::pin!(work);
    loop {
        tokio::select! {
            biased;
            result = &mut work => {
                let elapsed_ms = started.elapsed().as_millis();
                if stop.is_cancelled() {
                    tracing::info!(phase, elapsed_ms, "startup phase cancelled");
                } else if result.is_ok() {
                    tracing::info!(phase, elapsed_ms, "startup phase completed");
                } else {
                    tracing::error!(phase, elapsed_ms, "startup phase failed");
                }
                return result.map_err(Into::into);
            }
            _ = progress.tick() => {
                tracing::info!(phase, elapsed_ms = started.elapsed().as_millis(), "startup still in progress");
            }
        }
    }
}

/// Listen before startup is polled. A signal requests cancellation, then awaits
/// the same work so database operations and service cleanup can finish safely.
pub async fn until_shutdown<T>(
    stop: tokio_util::sync::CancellationToken,
    work: impl Future<Output = anyhow::Result<T>>,
    signal: impl Future<Output = std::io::Result<()>>,
) -> anyhow::Result<T> {
    let _cancel = stop.clone().drop_guard();
    tokio::pin!(work);
    tokio::select! { biased;
        signal = signal => {
            stop.cancel();
            let result = work.await;
            signal?;
            result
        }
        result = &mut work => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };
    use tracing::instrument::WithSubscriber;
    #[derive(Clone, Default)]
    struct Output(Arc<Mutex<Vec<u8>>>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn subscriber(output: Output) -> impl tracing::Subscriber + Send + Sync {
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || output.clone())
            .finish()
    }
    #[tokio::test(start_paused = true)]
    async fn slow_startup_reports_progress_and_completion_timing() {
        let output = Output::default();
        let result = phase(
            "restore",
            &tokio_util::sync::CancellationToken::new(),
            async {
                tokio::time::sleep(Duration::from_secs(11)).await;
                Ok::<_, std::io::Error>(42)
            },
        )
        .with_subscriber(subscriber(output.clone()))
        .await;
        assert_eq!(result.unwrap(), 42);
        let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        assert_eq!(text.matches("startup phase started").count(), 1);
        assert_eq!(text.matches("startup still in progress").count(), 2);
        assert!(text.contains("elapsed_ms=5000"));
        assert!(text.contains("elapsed_ms=10000"));
        assert!(text.contains("elapsed_ms=11000"));
        assert_eq!(text.matches("startup phase completed").count(), 1);
        assert!(!text.contains("startup phase failed"));
    }
    #[tokio::test]
    async fn failed_startup_preserves_the_error_and_does_not_report_success() {
        let output = Output::default();
        let result = phase(
            "database",
            &tokio_util::sync::CancellationToken::new(),
            std::future::ready(Err::<(), _>(std::io::Error::other("test failure"))),
        )
        .with_subscriber(subscriber(output.clone()))
        .await;
        assert_eq!(result.unwrap_err().to_string(), "test failure");
        let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        assert!(text.contains("startup phase failed"));
        assert!(!text.contains("startup still in progress"));
        assert!(!text.contains("startup phase completed"));
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use tokio_util::sync::CancellationToken;
    #[tokio::test]
    async fn signal_cancels_startup_and_waits_for_cleanup() {
        let stop = CancellationToken::new();
        let work_stop = stop.clone();
        let (release, wait) = tokio::sync::oneshot::channel();
        let mut task = tokio::spawn(until_shutdown(
            stop.clone(),
            async move {
                work_stop.cancelled().await;
                wait.await.unwrap();
                Ok(42)
            },
            async { Ok(()) },
        ));
        stop.cancelled().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut task)
                .await
                .is_err()
        );
        release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap(), 42);
    }
    #[tokio::test]
    async fn pre_cancelled_phase_never_polls_work() {
        let stop = CancellationToken::new();
        stop.cancel();
        let result = phase("never", &stop, async {
            panic!("cancelled startup polled work");
            #[allow(unreachable_code)]
            Ok::<(), std::io::Error>(())
        })
        .await;
        assert!(result.is_err());
    }
    #[tokio::test]
    async fn shutdown_does_not_hide_cleanup_errors() {
        let stop = CancellationToken::new();
        let worker = stop.clone();
        let result = until_shutdown(
            stop,
            async move {
                worker.cancelled().await;
                Err::<(), _>(anyhow::anyhow!("cleanup failed"))
            },
            async { Ok(()) },
        )
        .await;
        assert_eq!(result.unwrap_err().to_string(), "cleanup failed");
    }
}
