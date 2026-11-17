//! Bounded validated media caching, prioritized downloads and lifecycle management.
mod asset;
mod cache;
mod disk;
mod limits;
mod scheduler;
mod tgs;
pub use asset::{
    DocumentId, MAX_ANIMATION_BYTES, MAX_TGS_JSON_BYTES, MAX_THUMBNAIL_BYTES, MediaAsset,
    MediaError, MediaKey, MediaKind,
};
pub use cache::{MediaCache, MediaStats};
pub use limits::MediaLimits;
pub use scheduler::{MediaDownloader, MediaPriority};
/// Transport-independent result from a media source.
pub type MediaResult = Result<MediaAsset, MediaError>;
#[cfg(test)]
mod tests;
