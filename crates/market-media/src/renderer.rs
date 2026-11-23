//! One disposable renderer protocol for SVG previews and PNG covers.
use crate::{MediaAsset, MediaCache, MediaError, MediaResult};
use std::{io::Read, process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;
impl MediaCache {
    pub(crate) async fn render_derivative(
        &self,
        source: Vec<u8>,
        input: &'static str,
        output: &'static str,
        job: &crate::scheduler::Job,
    ) -> MediaResult {
        if self.0.stop.is_cancelled() || !job.has_demand() {
            return Err(MediaError::Unavailable);
        }
        let directory = self
            .blocking(move || {
                let directory = tempfile::tempdir().map_err(|_| MediaError::Unavailable)?;
                std::fs::write(directory.path().join(input), source)
                    .map_err(|_| MediaError::Unavailable)?;
                Ok(directory)
            })
            .await?;
        if self.0.stop.is_cancelled() || !job.has_demand() {
            return Err(MediaError::Unavailable);
        }
        let mut child = tokio::process::Command::new(&self.0.renderer)
            .arg(directory.path().join(input))
            .arg(directory.path().join(output))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                tracing::warn!(
                    document_id = job.key.document().get(), variant = ?job.key.kind(),
                    stage = "derivative_spawn",
                    ?error,
                    "media renderer could not start"
                );
                MediaError::Unavailable
            })?;
        let mut stderr = child.stderr.take().ok_or(MediaError::Unavailable)?;
        // Drain the pipe concurrently, retaining only a bounded diagnostic prefix.
        let diagnostics = async {
            let mut saved = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let count = stderr.read(&mut buffer).await.unwrap_or(0);
                if count == 0 {
                    break;
                }
                let keep = count.min(4096usize.saturating_sub(saved.len()));
                saved.extend_from_slice(&buffer[..keep]);
            }
            saved
        };
        let execution = async { tokio::join!(child.wait(), diagnostics) };
        let status = tokio::select! { biased;
            _ = self.0.stop.cancelled() => None,
            _ = job.abandoned() => None,
            result = tokio::time::timeout(Duration::from_secs(10), execution) => result.ok(),
        };
        let Some((status, diagnostics)) = status else {
            let _ = child.kill().await;
            let _ = child.wait().await;
            if !self.0.stop.is_cancelled() && job.has_demand() {
                tracing::warn!(
                    document_id = job.key.document().get(), variant = ?job.key.kind(),
                    stage = "derivative_render",
                    "media renderer timed out"
                );
            }
            return Err(MediaError::Unavailable);
        };
        let status = status.map_err(|_| MediaError::Unavailable)?;
        if !status.success() {
            tracing::warn!(document_id = job.key.document().get(), variant = ?job.key.kind(), stage = "derivative_render", ?status,
                diagnostic = %String::from_utf8_lossy(&diagnostics), "media renderer failed");
            // Exit 2 is the helper protocol for unsupported/over-budget input.
            return Err(if status.code() == Some(2) {
                MediaError::InvalidMedia
            } else {
                MediaError::Unavailable
            });
        }
        let kind = job.key.kind();
        let max_bytes = job.key.max_bytes();
        self.blocking(move || {
            let file = std::fs::File::open(directory.path().join(output))
                .map_err(|_| MediaError::Unavailable)?;
            let mut bytes = Vec::new();
            file.take(max_bytes as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| MediaError::Unavailable)?;
            MediaAsset::parse(kind, bytes)
        })
        .await
    }
}
