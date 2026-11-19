//! Derived SVGs share source downloads, cache quotas, admission and shutdown with other media.
use crate::{
    DocumentId, MAX_PREVIEW_BYTES, MediaAsset, MediaCache, MediaError, MediaKey, MediaPriority,
    MediaResult, scheduler::Running,
};
use std::{io::Read, process::Stdio, sync::atomic::Ordering, time::Duration};
use tokio::io::AsyncReadExt;
#[cfg(test)]
mod tests;

pub(crate) fn validate(bytes: &[u8]) -> Result<(), MediaError> {
    let invalid = MediaError::InvalidMedia;
    if bytes.is_empty() || bytes.len() > MAX_PREVIEW_BYTES {
        return Err(invalid);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid)?;
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            nodes_limit: 50_000,
            ..Default::default()
        },
    )
    .map_err(|_| invalid)?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" || root.attribute("viewBox") != Some("0 0 512 512") {
        return Err(invalid);
    }
    for node in doc.descendants().filter(|node| node.is_element()) {
        if node.tag_name().namespace() != Some("http://www.w3.org/2000/svg")
            || !matches!(
                node.tag_name().name(),
                "svg"
                    | "defs"
                    | "g"
                    | "path"
                    | "rect"
                    | "linearGradient"
                    | "radialGradient"
                    | "stop"
                    | "mask"
                    | "clipPath"
                    | "filter"
                    | "feGaussianBlur"
                    | "feColorMatrix"
            )
        {
            return Err(invalid);
        }
        for attr in node.attributes() {
            if attr.namespace().is_some()
                || attr.name().starts_with("on")
                || matches!(attr.name(), "href" | "src")
            {
                return Err(invalid);
            }
            let value = attr.value();
            if attr.name() == "type" && value != "matrix" {
                return Err(invalid);
            }
            if !matches!(
                attr.name(),
                "type"
                    | "values"
                    | "width"
                    | "height"
                    | "viewBox"
                    | "id"
                    | "x"
                    | "y"
                    | "x1"
                    | "y1"
                    | "x2"
                    | "y2"
                    | "cx"
                    | "cy"
                    | "r"
                    | "fx"
                    | "fy"
                    | "fr"
                    | "rx"
                    | "ry"
                    | "d"
                    | "transform"
                    | "gradientTransform"
                    | "gradientUnits"
                    | "spreadMethod"
                    | "offset"
                    | "stop-color"
                    | "stop-opacity"
                    | "fill"
                    | "fill-rule"
                    | "fill-opacity"
                    | "stroke"
                    | "stroke-width"
                    | "stroke-linecap"
                    | "stroke-linejoin"
                    | "stroke-miterlimit"
                    | "stroke-dasharray"
                    | "stroke-dashoffset"
                    | "stroke-opacity"
                    | "paint-order"
                    | "opacity"
                    | "clip-path"
                    | "clip-rule"
                    | "clipPathUnits"
                    | "mask"
                    | "mask-type"
                    | "maskUnits"
                    | "maskContentUnits"
                    | "filter"
                    | "filterUnits"
                    | "stdDeviation"
                    | "color-interpolation-filters"
                    | "style"
            ) {
                return Err(invalid);
            }
            if matches!(
                attr.name(),
                "fill" | "stroke" | "stop-color" | "mask" | "clip-path" | "filter"
            ) && value != "none"
                && !(value.starts_with('#') && value[1..].chars().all(|c| c.is_ascii_hexdigit()))
                && !value.starts_with("url(#")
            {
                return Err(invalid);
            }
            if value.contains("url(")
                && !(value.starts_with("url(#")
                    && value.ends_with(')')
                    && value[5..value.len() - 1]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            {
                return Err(invalid);
            }
            if attr.name() == "style"
                && !matches!(
                    value.trim_end_matches(';'),
                    "mix-blend-mode:normal"
                        | "mix-blend-mode:multiply"
                        | "mix-blend-mode:screen"
                        | "mix-blend-mode:overlay"
                        | "mix-blend-mode:darken"
                        | "mix-blend-mode:lighten"
                        | "mix-blend-mode:color-dodge"
                        | "mix-blend-mode:color-burn"
                        | "mix-blend-mode:hard-light"
                        | "mix-blend-mode:soft-light"
                        | "mix-blend-mode:difference"
                        | "mix-blend-mode:exclusion"
                        | "mix-blend-mode:hue"
                        | "mix-blend-mode:saturation"
                        | "mix-blend-mode:color"
                        | "mix-blend-mode:luminosity"
                )
            {
                return Err(invalid);
            }
        }
    }
    Ok(())
}

impl MediaCache {
    pub async fn preview(&self, document: DocumentId) -> MediaResult {
        if self.0.closed.load(Ordering::Acquire) {
            return Err(MediaError::Unavailable);
        }
        // Source identity is indexed separately from resident TGS bytes.
        let cached_job = if let Some(key) = self.preview_key(document) {
            if let Some(asset) = self.resident(key) {
                return Ok(asset);
            }
            let job = self.job(key)?;
            if let Some(asset) = self.disk(&job).await? {
                return Ok(asset);
            }
            if let Some(error) = self.failure(key) {
                return Err(error);
            }
            Some(job)
        } else {
            None
        };
        // Resolve the source before derivative admission; downloads never wait on their own pool.
        let source = self.get(MediaKey::animation(document)).await?;
        let key = MediaKey::preview(document, source.digest);
        if let Some(asset) = self.resident(key) {
            return Ok(asset);
        }
        let job = match cached_job.filter(|job| job.key == key) {
            Some(job) => job,
            None => self.job(key)?,
        };
        let _demand = job.demand(MediaPriority::Interactive);
        if let Some(asset) = self.disk(&job).await? {
            return Ok(asset);
        }
        if let Some(error) = self.failure(key) {
            return Err(error);
        }
        if !job.queued.swap(true, Ordering::AcqRel) {
            let _admission = self.0.admission.lock().unwrap_or_else(|e| e.into_inner());
            if self.0.closed.load(Ordering::Acquire) {
                return Err(MediaError::Unavailable);
            }
            let cache = self.clone();
            let job = job.clone();
            self.0.tasks.spawn(async move {
                let guard = Running::new(cache.clone(), job.clone());
                let work = async {
                    let permit = tokio::select! { biased;
                        _ = cache.0.stop.cancelled() => return Err(MediaError::Unavailable),
                        _ = job.abandoned() => return Err(MediaError::Unavailable),
                        permit = cache.0.preview_slots.clone().acquire_owned() => permit.map_err(|_| MediaError::Unavailable)?,
                    };
                    let _permit = permit;
                    cache.0.counters.renders.fetch_add(1, Ordering::Relaxed);
                    let asset = cache.render_preview(source, &job).await?;
                    cache.store(key, asset).await
                };
                let result = work.await;
                guard.finish(result);
            });
        }
        job.wait(self.0.limits.request_timeout).await
    }

    async fn render_preview(&self, source: MediaAsset, job: &crate::scheduler::Job) -> MediaResult {
        let directory = self
            .blocking(move || {
                let directory = tempfile::tempdir().map_err(|_| MediaError::Unavailable)?;
                std::fs::write(directory.path().join("source.tgs"), source.bytes())
                    .map_err(|_| MediaError::Unavailable)?;
                Ok(directory)
            })
            .await?;
        if self.0.stop.is_cancelled() {
            return Err(MediaError::Unavailable);
        }
        let mut child = tokio::process::Command::new(&self.0.renderer)
            .arg(directory.path().join("source.tgs"))
            .arg(directory.path().join("preview.svg"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                tracing::warn!(
                    document_id = job.key.document().get(),
                    stage = "preview_spawn",
                    ?error,
                    "preview renderer could not start"
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
                    document_id = job.key.document().get(),
                    stage = "preview_render",
                    "preview renderer timed out"
                );
            }
            return Err(MediaError::Unavailable);
        };
        let status = status.map_err(|_| MediaError::Unavailable)?;
        if !status.success() {
            tracing::warn!(document_id = job.key.document().get(), stage = "preview_render", ?status,
                diagnostic = %String::from_utf8_lossy(&diagnostics), "preview renderer failed");
            // Exit 2 is the helper protocol for unsupported/over-budget input.
            return Err(if status.code() == Some(2) {
                MediaError::InvalidMedia
            } else {
                MediaError::Unavailable
            });
        }
        self.blocking(move || {
            let file = std::fs::File::open(directory.path().join("preview.svg"))
                .map_err(|_| MediaError::Unavailable)?;
            let mut bytes = Vec::new();
            file.take(MAX_PREVIEW_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| MediaError::Unavailable)?;
            MediaAsset::preview(bytes)
        })
        .await
    }
}
