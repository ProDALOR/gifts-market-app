//! Scoped startup diagnostics: slow work is visible without a detached ticker task.
use std::{future::Future, time::Duration};
use tokio::time::{Instant, MissedTickBehavior, interval_at};

pub async fn phase<T, E>(
    phase: &'static str,
    work: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
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
                if result.is_ok() {
                    tracing::info!(phase, elapsed_ms, "startup phase completed");
                } else {
                    tracing::error!(phase, elapsed_ms, "startup phase failed");
                }
                return result;
            }
            _ = progress.tick() => {
                tracing::info!(phase, elapsed_ms = started.elapsed().as_millis(), "startup still in progress");
            }
        }
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
        let result = phase("restore", async {
            tokio::time::sleep(Duration::from_secs(11)).await;
            Ok::<_, ()>(42)
        })
        .with_subscriber(subscriber(output.clone()))
        .await;
        assert_eq!(result, Ok(42));
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
        let result = phase("database", std::future::ready(Err::<(), _>(7)))
            .with_subscriber(subscriber(output.clone()))
            .await;
        assert_eq!(result, Err(7));
        let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        assert!(text.contains("startup phase failed"));
        assert!(!text.contains("startup still in progress"));
        assert!(!text.contains("startup phase completed"));
    }
}
