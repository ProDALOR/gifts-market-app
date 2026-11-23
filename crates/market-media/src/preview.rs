//! Derived SVGs share source downloads, cache quotas, admission and shutdown with other media.
use crate::{
    DocumentId, MAX_PREVIEW_BYTES, MediaAsset, MediaCache, MediaError, MediaKey, MediaResult,
};
use std::sync::atomic::Ordering;
#[cfg(test)]
mod tests;

pub(crate) fn validate(bytes: &[u8]) -> Result<(), MediaError> {
    validate_svg(bytes, "0 0 512 512", MAX_PREVIEW_BYTES)
}
pub(crate) fn validate_svg(
    bytes: &[u8],
    view_box: &str,
    max_bytes: usize,
) -> Result<(), MediaError> {
    let invalid = MediaError::InvalidMedia;
    if bytes.is_empty() || bytes.len() > max_bytes {
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
    if root.tag_name().name() != "svg" || root.attribute("viewBox") != Some(view_box) {
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
        self.preview_snapshot(document)
            .await
            .map(|(_, asset)| asset)
    }
    /// Keep source identity paired with the returned SVG across concurrent source replacements.
    pub(crate) async fn preview_snapshot(
        &self,
        document: DocumentId,
    ) -> Result<(MediaKey, MediaAsset), MediaError> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err(MediaError::Unavailable);
        }
        // Retain the coalesced disk result while resolving a possible source miss.
        let _cached_job = if let Some(key) = self.preview_key(document) {
            if let Some(asset) = self.resident(key) {
                return Ok((key, asset));
            }
            let job = self.job(key)?;
            if let Some(asset) = self.disk(&job).await? {
                return Ok((key, asset));
            }
            if let Some(error) = self.failure(key) {
                return Err(error);
            }
            Some(job)
        } else {
            None
        };
        // Source resolution must not hold a derivative renderer slot.
        let source = self.get(MediaKey::animation(document)).await?;
        let key = MediaKey::preview(document, source.digest);
        let asset = self
            .derive(key, move |cache, job| async move {
                cache
                    .render_derivative(source.bytes().to_vec(), "source.tgs", "preview.svg", &job)
                    .await
            })
            .await?;
        Ok((key, asset))
    }
}
