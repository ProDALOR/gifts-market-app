//! Bounded validated media caching, prioritized downloads and lifecycle management.
mod asset;
mod cache;
mod disk;
mod limits;
mod placeholder;
mod preview;
mod scheduler;
mod svg;
mod tgs;
pub use asset::{
    DocumentId, MAX_ANIMATION_BYTES, MAX_PLACEHOLDER_BYTES, MAX_PREVIEW_BYTES, MAX_TGS_JSON_BYTES,
    MAX_THUMBNAIL_BYTES, MediaAsset, MediaError, MediaKey, MediaKind, PREVIEW_VERSION,
};
pub use cache::{MediaCache, MediaStats};
pub use limits::MediaLimits;
pub use scheduler::{MediaDownloader, MediaPriority};
pub use svg::first_frame_svg;
/// Transport-independent result from a media source.
pub type MediaResult = Result<MediaAsset, MediaError>;
#[cfg(test)]
mod tests;
