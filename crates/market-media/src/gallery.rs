//! A bounded page of gift artwork, cached by sources, visible labels and layout version.
use crate::*;
use futures_util::{StreamExt, TryStreamExt};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

pub const MAX_GALLERY_CELLS: usize = 12;
pub const MAX_GALLERY_SVG_BYTES: usize = 32 * MAX_PREVIEW_BYTES;
const WIDTH: u32 = 1200;
const CELL: u32 = 384;
const FONT: &[u8] = include_bytes!("../assets/gallery-font.ttf");
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GalleryCell {
    pub cover: Option<GiftCover>,
    pub title: String,
    pub identity: String,
    pub price: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GiftGallery {
    pub columns: u8,
    pub cells: Vec<GalleryCell>,
}
impl GiftGallery {
    fn validate(&self) -> Result<(), MediaError> {
        if self.cells.is_empty()
            || self.cells.len() > MAX_GALLERY_CELLS
            || !(2..=3).contains(&self.columns)
            || self.cells.len().div_ceil(usize::from(self.columns)) > 4
        {
            return Err(MediaError::InvalidMedia);
        }
        for cell in &self.cells {
            if [&cell.title, &cell.identity, &cell.price]
                .iter()
                .any(|s| s.len() > 256 || s.chars().any(char::is_control))
                || cell
                    .cover
                    .and_then(|cover| cover.palette)
                    .is_some_and(|p| [p.center, p.edge, p.pattern].iter().any(|c| *c > 0xffffff))
            {
                return Err(MediaError::InvalidMedia);
            }
        }
        Ok(())
    }
    pub fn document(&self) -> Option<DocumentId> {
        self.cells
            .iter()
            .find_map(|cell| cell.cover.map(|cover| cover.model))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Artwork {
    Gift(GiftCover),
    Gallery(GiftGallery),
}
impl From<GiftCover> for Artwork {
    fn from(value: GiftCover) -> Self {
        Self::Gift(value)
    }
}
impl Artwork {
    pub fn document(&self) -> Option<DocumentId> {
        match self {
            Self::Gift(cover) => Some(cover.model),
            Self::Gallery(gallery) => gallery.document(),
        }
    }
}
impl MediaCache {
    pub async fn gallery(&self, mut gallery: GiftGallery) -> MediaResult {
        if self.0.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(MediaError::Unavailable);
        }
        gallery.validate()?;
        for cell in &mut gallery.cells {
            cell.cover = cell.cover.map(GiftCover::normalized);
        }
        let document = gallery
            .document()
            .unwrap_or(UiIcon::Collection.definition().document);
        let ids: BTreeSet<_> = gallery
            .cells
            .iter()
            .filter_map(|cell| cell.cover)
            .flat_map(|cover| std::iter::once(cover.model).chain(cover.pattern))
            .collect();
        let sources: BTreeMap<_, _> = futures_util::stream::iter(ids.iter().copied())
            .map(|id| async move { self.cover_source(id).await.map(|source| (id, source)) })
            .buffered(4)
            .try_collect()
            .await?;
        let candidate = key(document, &gallery, &sources);
        if let Some(asset) = self.resident(candidate) {
            return Ok(asset);
        }
        let job = self.job(candidate)?;
        if let Some(asset) = self.disk(&job).await? {
            return Ok(asset);
        }
        if let Some(error) = self.failure(candidate) {
            return Err(error);
        }
        let previews: Vec<_> = futures_util::stream::iter(ids)
            .map(|id| async move {
                self.preview_snapshot(id)
                    .await
                    .map(|(key, asset)| (id, key.source().expect("preview source"), asset))
            })
            .buffered(4)
            .try_collect()
            .await?;
        let actual = previews
            .iter()
            .map(|(id, source, _)| (*id, *source))
            .collect();
        let key = key(document, &gallery, &actual);
        let assets = previews
            .into_iter()
            .map(|(id, _, asset)| (id, asset))
            .collect();
        self.derive(key, move |cache, job| async move {
            let svg = cache.blocking(move || compose(&gallery, &assets)).await?;
            cache
                .render_derivative(svg.into_bytes(), "gallery.svg", "gallery.png", &job)
                .await
        })
        .await
    }
}
fn key(
    document: DocumentId,
    gallery: &GiftGallery,
    sources: &BTreeMap<DocumentId, [u8; 32]>,
) -> MediaKey {
    let mut hash = Sha256::new();
    hash.update(crate::asset::GALLERY_VERSION);
    hash.update(crate::asset::COVER_VERSION);
    hash.update(crate::PREVIEW_VERSION);
    hash.update(include_bytes!("../assets/gift-pattern.json"));
    hash.update(FONT);
    hash.update([gallery.columns]);
    for cell in &gallery.cells {
        hash.update([u8::from(cell.cover.is_some())]);
        if let Some(cover) = cell.cover {
            hash.update(cover.model.get().to_be_bytes());
            hash.update(cover.pattern.map_or(0, DocumentId::get).to_be_bytes());
            hash.update([u8::from(cover.palette.is_some())]);
            if let Some(p) = cover.palette {
                for color in [p.center, p.edge, p.pattern] {
                    hash.update(color.to_be_bytes());
                }
            }
        }
        for value in [&cell.title, &cell.identity, &cell.price] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value);
        }
    }
    for (id, source) in sources {
        hash.update(id.get().to_be_bytes());
        hash.update(source);
    }
    MediaKey::gallery(document, hash.finalize().into())
}
struct Outline(String);
impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "M{x} {y}");
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "L{x} {y}");
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let _ = write!(self.0, "Q{x1} {y1} {x} {y}");
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let _ = write!(self.0, "C{x1} {y1} {x2} {y2} {x} {y}");
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}
fn text(value: &str, x: f32, y: f32, size: f32, max_width: f32) -> Result<String, MediaError> {
    let face = ttf_parser::Face::parse(FONT, 0).map_err(|_| MediaError::InvalidMedia)?;
    let glyphs: Vec<_> = value
        .chars()
        .take(40)
        .filter_map(|c| face.glyph_index(c).or_else(|| face.glyph_index('□')))
        .collect();
    let width: f32 = glyphs
        .iter()
        .map(|g| f32::from(face.glyph_hor_advance(*g).unwrap_or(0)))
        .sum();
    let scale = (size / f32::from(face.units_per_em())).min(max_width / width.max(1.0));
    let mut out = format!(
        r##"<g fill="#ffffff" transform="translate({x} {y}) scale({scale} {})">"##,
        -scale
    );
    let mut position = 0.0;
    for glyph in glyphs {
        let mut path = Outline(String::new());
        face.outline_glyph(glyph, &mut path);
        out.push_str(&format!(
            r#"<path transform="translate({position} 0)" d="{}"/>"#,
            path.0
        ));
        position += f32::from(face.glyph_hor_advance(glyph).unwrap_or(0));
    }
    out.push_str("</g>");
    Ok(out)
}
fn cell_size(columns: u8) -> u32 {
    (WIDTH - 12 * (u32::from(columns) + 1)) / u32::from(columns)
}
fn height(count: usize, columns: u8) -> u32 {
    (count.div_ceil(usize::from(columns)) as u32) * (cell_size(columns) + 12) + 12
}
fn compose(
    gallery: &GiftGallery,
    assets: &BTreeMap<DocumentId, MediaAsset>,
) -> Result<String, MediaError> {
    gallery.validate()?;
    let height = height(gallery.cells.len(), gallery.columns);
    let mut out = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}"><defs><linearGradient id="shade" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#101014" stop-opacity="0"/><stop offset="1" stop-color="#101014" stop-opacity="0.8"/></linearGradient></defs><rect width="{WIDTH}" height="{height}" fill="#111115"/>"##
    );
    for (i, cell) in gallery.cells.iter().enumerate() {
        let step = cell_size(gallery.columns) + 12;
        let x = 12 + (i as u32 % u32::from(gallery.columns)) * step;
        let y = 12 + (i as u32 / u32::from(gallery.columns)) * step;
        let scale = cell_size(gallery.columns) as f32 / CELL as f32;
        out.push_str(&format!(r#"<defs><clipPath id="cell-{i}"><rect width="{CELL}" height="{CELL}" rx="22"/></clipPath></defs><g transform="translate({x} {y}) scale({scale})" clip-path="url(#cell-{i})">"#));
        if let Some(cover) = cell.cover {
            let model = assets.get(&cover.model).ok_or(MediaError::InvalidMedia)?;
            let pattern = cover.pattern.and_then(|id| assets.get(&id));
            let svg = crate::cover::compose_sized(model, pattern, cover.palette, CELL, CELL)?;
            let svg = crate::cover::scoped_svg(&svg, &format!("c{i}"));
            out.push_str(&svg);
        } else {
            out.push_str(r##"<rect width="384" height="384" fill="#34303c"/>"##);
        }
        out.push_str(r##"<rect y="230" width="384" height="154" fill="url(#shade)"/><rect x="12" y="12" width="175" height="35" rx="10" fill="#16151c" opacity="0.6"/>"##);
        out.push_str(&text(&cell.identity, 22.0, 37.0, 22.0, 155.0)?);
        out.push_str(&text(&cell.title, 20.0, 326.0, 25.0, 344.0)?);
        out.push_str(&text(&cell.price, 20.0, 363.0, 30.0, 344.0)?);
        out.push_str("</g>");
        if out.len() > MAX_GALLERY_SVG_BYTES {
            return Err(MediaError::InvalidMedia);
        }
    }
    out.push_str("</svg>");
    Ok(out)
}
pub fn render_gallery_png(svg: &[u8]) -> Result<Vec<u8>, MediaError> {
    if svg.len() > MAX_GALLERY_SVG_BYTES {
        return Err(MediaError::InvalidMedia);
    }
    let tree = roxmltree::Document::parse_with_options(
        std::str::from_utf8(svg).map_err(|_| MediaError::InvalidMedia)?,
        roxmltree::ParsingOptions {
            nodes_limit: 50_000,
            ..Default::default()
        },
    )
    .map_err(|_| MediaError::InvalidMedia)?;
    let height: u32 = tree
        .root_element()
        .attribute("height")
        .ok_or(MediaError::InvalidMedia)?
        .parse()
        .map_err(|_| MediaError::InvalidMedia)?;
    if !(2..=3).any(|columns| (1..=4).any(|rows| height == height_for_rows(rows, columns))) {
        return Err(MediaError::InvalidMedia);
    }
    crate::preview::validate_svg(svg, &format!("0 0 {WIDTH} {height}"), MAX_GALLERY_SVG_BYTES)?;
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .map_err(|_| MediaError::InvalidMedia)?;
    if tree.size().width() != WIDTH as f32 || tree.size().height() != height as f32 {
        return Err(MediaError::InvalidMedia);
    }
    let mut pixels = resvg::tiny_skia::Pixmap::new(WIDTH, height).ok_or(MediaError::Unavailable)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixels.as_mut(),
    );
    pixels.encode_png().map_err(|_| MediaError::InvalidMedia)
}
fn height_for_rows(rows: u32, columns: u8) -> u32 {
    rows * (cell_size(columns) + 12) + 12
}
pub(crate) fn validate_png(bytes: &[u8]) -> Result<(), MediaError> {
    if bytes.len() > MAX_COVER_BYTES
        || bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n")
        || bytes.get(16..20) != Some(WIDTH.to_be_bytes().as_slice())
    {
        return Err(MediaError::InvalidMedia);
    }
    let h = bytes
        .get(20..24)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or(MediaError::InvalidMedia)?;
    if !(2..=3).any(|columns| (1..=4).any(|r| h == height_for_rows(r, columns))) {
        return Err(MediaError::InvalidMedia);
    }
    resvg::tiny_skia::Pixmap::decode_png(bytes)
        .map(|_| ())
        .map_err(|_| MediaError::InvalidMedia)
}

#[cfg(test)]
mod tests;
