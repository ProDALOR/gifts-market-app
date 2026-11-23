//! Bounded validated media caching, paced downloads and lifecycle management.
mod asset;
mod cache;
mod cover;
pub use cover::{COVER_HEIGHT, COVER_WIDTH, CoverPalette, GiftCover, render_cover_png};
mod disk;
mod limits;
mod preview;
mod renderer;
mod scheduler;
mod svg;
mod tgs;
pub use asset::{
    DocumentId, MAX_ANIMATION_BYTES, MAX_COVER_BYTES, MAX_PREVIEW_BYTES, MAX_TGS_JSON_BYTES,
    MAX_THUMBNAIL_BYTES, MediaAsset, MediaError, MediaKey, MediaKind, PREVIEW_VERSION,
};
pub use cache::{MediaCache, MediaStats};
pub use limits::MediaLimits;
pub use scheduler::MediaDownloader;
pub use svg::first_frame_svg;
/// Transport-independent result from a media source.
pub type MediaResult = Result<MediaAsset, MediaError>;
#[cfg(test)]
mod tests;
