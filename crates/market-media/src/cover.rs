//! Content-addressed gift covers. All sources use the existing SVG cache and renderer budgets.
use crate::{DocumentId, MediaAsset, MediaCache, MediaError, MediaKey, MediaResult};
use sha2::{Digest, Sha256};
pub const COVER_WIDTH: u32 = 1200;
pub const COVER_HEIGHT: u32 = 600;
const VIEW_WIDTH: f64 = 420.0;
const VIEW_HEIGHT: f64 = VIEW_WIDTH * COVER_HEIGHT as f64 / COVER_WIDTH as f64;
fn view_box() -> String {
    format!("0 0 {VIEW_WIDTH} {VIEW_HEIGHT}")
}
const MAX_SVG_BYTES: usize = 3 * crate::MAX_PREVIEW_BYTES;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CoverPalette {
    pub center: u32,
    pub edge: u32,
    pub pattern: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GiftCover {
    pub model: DocumentId,
    pub pattern: Option<DocumentId>,
    pub palette: Option<CoverPalette>,
}
impl GiftCover {
    pub(crate) fn normalized(mut self) -> Self {
        if self.palette.is_none() {
            self.pattern = None;
        }
        if self.pattern.is_none()
            && let Some(palette) = &mut self.palette
        {
            palette.pattern = 0;
        }
        self
    }
}
impl MediaCache {
    pub async fn cover(&self, cover: GiftCover) -> MediaResult {
        if cover
            .palette
            .is_some_and(|p| [p.center, p.edge, p.pattern].iter().any(|v| *v > 0xffffff))
        {
            return Err(MediaError::InvalidMedia);
        }
        if self.0.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(MediaError::Unavailable);
        }
        let cover = cover.normalized();
        // Current source identities can select a cached PNG without loading or rebuilding SVGs.
        let (model_source, pattern_source) =
            tokio::try_join!(self.cover_source(cover.model), async {
                match cover.pattern {
                    Some(id) => self.cover_source(id).await.map(Some),
                    None => Ok(None),
                }
            })?;
        let candidate = cover_key(cover, model_source, pattern_source);
        if let Some(asset) = self.resident(candidate) {
            return Ok(asset);
        }
        // Retain the coalesced disk result if this same key proceeds to rendering.
        let candidate_job = self.job(candidate)?;
        if let Some(asset) = self.disk(&candidate_job).await? {
            return Ok(asset);
        }
        if let Some(error) = self.failure(candidate) {
            return Err(error);
        }
        // Dependencies are resolved outside the render pool, and carry their actual source keys.
        let ((model_key, model), pattern) =
            tokio::try_join!(self.preview_snapshot(cover.model), async {
                match cover.pattern {
                    Some(id) => self.preview_snapshot(id).await.map(Some),
                    None => Ok(None),
                }
            })?;
        let key = cover_key(
            cover,
            model_key.source().expect("preview source"),
            pattern
                .as_ref()
                .map(|(key, _)| key.source().expect("preview source")),
        );
        self.derive(key, move |cache, job| async move {
            let svg = cache
                .blocking(move || {
                    compose_sized(
                        &model,
                        pattern.as_ref().map(|(_, asset)| asset),
                        cover.palette,
                        COVER_WIDTH,
                        COVER_HEIGHT,
                    )
                })
                .await?;
            cache
                .render_derivative(svg.into_bytes(), "cover.svg", "cover.png", &job)
                .await
        })
        .await
    }
    pub(crate) async fn cover_source(&self, document: DocumentId) -> Result<[u8; 32], MediaError> {
        if let Some(key) = self.preview_key(document) {
            return Ok(key.source().expect("preview source"));
        }
        // After restart, retained TGS files are enough to find a cover even if its SVGs were evicted.
        self.get(MediaKey::animation(document))
            .await
            .map(|asset| asset.digest)
    }
}
fn cover_key(
    cover: GiftCover,
    model_source: [u8; 32],
    pattern_source: Option<[u8; 32]>,
) -> MediaKey {
    let cover = cover.normalized();
    let mut hash = Sha256::new();
    hash.update(crate::asset::COVER_VERSION);
    hash.update(crate::PREVIEW_VERSION);
    hash.update(include_bytes!("../assets/gift-pattern.json"));
    hash.update(model_source);
    hash.update(
        pattern_source
            .filter(|_| cover.pattern.is_some())
            .unwrap_or([0; 32]),
    );
    hash.update(cover.pattern.map_or(0, DocumentId::get).to_be_bytes());
    hash.update([u8::from(cover.palette.is_some())]);
    if let Some(p) = cover.palette {
        for color in [p.center, p.edge, p.pattern] {
            hash.update(color.to_be_bytes());
        }
    }
    MediaKey::cover(cover.model, hash.finalize().into())
}
pub(crate) fn scoped(asset: &MediaAsset, prefix: &str) -> Result<String, MediaError> {
    let svg = std::str::from_utf8(asset.bytes()).map_err(|_| MediaError::InvalidMedia)?;
    Ok(scoped_svg(svg, prefix))
}
pub(crate) fn scoped_svg(svg: &str, prefix: &str) -> String {
    svg.replace("id=\"", &format!("id=\"{prefix}-"))
        .replace("id='", &format!("id='{prefix}-"))
        .replace("url(#", &format!("url(#{prefix}-"))
        .replace("href=\"#", &format!("href=\"#{prefix}-"))
}
pub(crate) fn compose_sized(
    model: &MediaAsset,
    pattern: Option<&MediaAsset>,
    palette: Option<CoverPalette>,
    width: u32,
    height: u32,
) -> Result<String, MediaError> {
    let p = palette.unwrap_or(CoverPalette {
        center: 0x41404c,
        edge: 0x24232c,
        pattern: 0x888692,
    });
    let view_height = VIEW_WIDTH * height as f64 / width as f64;
    let view_box = format!("0 0 {VIEW_WIDTH} {view_height}");
    let center_x = VIEW_WIDTH / 2.0;
    let gradient_y = view_height * 0.42;
    let gradient_radius = VIEW_WIDTH.max(view_height) * 0.72;
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="{view_box}"><defs><radialGradient id="cover-bg" gradientUnits="userSpaceOnUse" cx="{center_x}" cy="{gradient_y}" r="{gradient_radius}"><stop offset="0" stop-color="#{:06x}"/><stop offset="1" stop-color="#{:06x}"/></radialGradient>"##,
        p.center, p.edge
    );
    if let Some(pattern) = pattern {
        svg.push_str(&format!(r#"<mask id="cover-pattern" mask-type="alpha" maskUnits="userSpaceOnUse" x="0" y="0" width="512" height="512">{}</mask>"#, scoped(pattern,"pattern")?));
    }
    svg.push_str(&format!(
        r##"</defs><rect width="{VIEW_WIDTH}" height="{view_height}" fill="url(#cover-bg)"/>"##
    ));
    if pattern.is_some() {
        // Same cover-fit rule as GUI giftPatternTiles: crop vertically, never stretch symbols.
        let top = (view_height - 280.0) / 2.0;
        svg.push_str(&format!(r#"<g transform="translate(0 {top})">"#));
        let positions: Vec<[f64; 4]> =
            serde_json::from_str(include_str!("../assets/gift-pattern.json"))
                .map_err(|_| MediaError::InvalidMedia)?;
        for [x, y, scale, opacity] in positions {
            svg.push_str(&format!(r##"<g transform="translate({x} {y}) scale({})" opacity="{opacity}"><rect width="512" height="512" fill="#{:06x}" mask="url(#cover-pattern)"/></g>"##,100.0*scale/512.0,p.pattern));
        }
        svg.push_str("</g>");
    }
    // GUI's decorated artwork occupies 76% of the shorter side, centered in both axes.
    let model_side = VIEW_WIDTH.min(view_height) * 0.76;
    let model_x = (VIEW_WIDTH - model_side) / 2.0;
    let model_y = (view_height - model_side) / 2.0;
    let model_scale = model_side / 512.0;
    svg.push_str(&format!(
        r#"<g transform="translate({model_x} {model_y}) scale({model_scale})">{}</g></svg>"#,
        scoped(model, "model")?
    ));
    // Inputs were validated by the SVG cache; validate the assembled SVG once in the helper.
    if svg.len() > MAX_SVG_BYTES {
        return Err(MediaError::InvalidMedia);
    }
    Ok(svg)
}
#[cfg(test)]
fn compose(
    model: &MediaAsset,
    pattern: Option<&MediaAsset>,
    palette: Option<CoverPalette>,
) -> Result<String, MediaError> {
    compose_sized(model, pattern, palette, COVER_WIDTH, COVER_HEIGHT)
}
/// Used by the disposable renderer process; SVGs cannot load external resources or fonts.
pub fn render_cover_png(svg: &[u8]) -> Result<Vec<u8>, MediaError> {
    crate::preview::validate_svg(svg, &view_box(), MAX_SVG_BYTES)?;
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .map_err(|_| MediaError::InvalidMedia)?;
    if tree.size().width() != COVER_WIDTH as f32 || tree.size().height() != COVER_HEIGHT as f32 {
        return Err(MediaError::InvalidMedia);
    }
    let mut pixels =
        resvg::tiny_skia::Pixmap::new(COVER_WIDTH, COVER_HEIGHT).ok_or(MediaError::Unavailable)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixels.as_mut(),
    );
    pixels.encode_png().map_err(|_| MediaError::InvalidMedia)
}
pub(crate) fn validate_png(bytes: &[u8]) -> Result<(), MediaError> {
    if bytes.len() > crate::MAX_COVER_BYTES
        || bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n")
        || bytes.get(16..20) != Some(COVER_WIDTH.to_be_bytes().as_slice())
        || bytes.get(20..24) != Some(COVER_HEIGHT.to_be_bytes().as_slice())
    {
        return Err(MediaError::InvalidMedia);
    }
    resvg::tiny_skia::Pixmap::decode_png(bytes)
        .map(|_| ())
        .map_err(|_| MediaError::InvalidMedia)
}

#[cfg(test)]
mod tests;
