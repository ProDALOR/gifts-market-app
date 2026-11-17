use sha2::{Digest, Sha256};
use std::sync::Arc;
pub const MAX_ANIMATION_BYTES: usize = 256 * 1024;
pub const MAX_TGS_JSON_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_THUMBNAIL_BYTES: usize = 256 * 1024;
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct DocumentId(i64);
impl DocumentId {
    pub fn new(id: i64) -> Option<Self> {
        (id > 0).then_some(Self(id))
    }
    pub fn get(self) -> i64 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum MediaKind {
    Thumbnail,
    Animation,
}
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct MediaKey {
    document: DocumentId,
    kind: MediaKind,
}
impl MediaKey {
    pub fn thumbnail(document: DocumentId) -> Self {
        Self {
            document,
            kind: MediaKind::Thumbnail,
        }
    }
    pub fn animation(document: DocumentId) -> Self {
        Self {
            document,
            kind: MediaKind::Animation,
        }
    }
    pub fn document(self) -> DocumentId {
        self.document
    }
    pub fn kind(self) -> MediaKind {
        self.kind
    }
    pub fn max_bytes(self) -> usize {
        match self.kind {
            MediaKind::Thumbnail => MAX_THUMBNAIL_BYTES,
            MediaKind::Animation => MAX_ANIMATION_BYTES,
        }
    }
    pub(crate) fn extension(self) -> &'static str {
        match self.kind {
            MediaKind::Thumbnail => "thumb",
            MediaKind::Animation => "tgs",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaError {
    NotFound,
    Unavailable,
    Busy,
    RetryAfter(u64),
    InvalidMedia,
}
#[derive(Clone, Debug)]
pub struct MediaAsset {
    kind: MediaKind,
    bytes: Arc<[u8]>,
    content_type: &'static str,
    etag: Arc<str>,
}
impl MediaAsset {
    pub fn thumbnail(bytes: Vec<u8>) -> Result<Self, MediaError> {
        if bytes.len() > MAX_THUMBNAIL_BYTES {
            return Err(MediaError::InvalidMedia);
        }
        let content_type = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(b"\xff\xd8\xff") {
            "image/jpeg"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            "image/webp"
        } else {
            return Err(MediaError::InvalidMedia);
        };
        let etag = format!("\"{:x}\"", Sha256::digest(&bytes));
        Ok(Self {
            kind: MediaKind::Thumbnail,
            bytes: bytes.into(),
            content_type,
            etag: etag.into(),
        })
    }
    pub fn animation(bytes: Vec<u8>) -> Result<Self, MediaError> {
        crate::tgs::validate(&bytes)?;
        Ok(Self {
            kind: MediaKind::Animation,
            etag: format!("\"{:x}\"", Sha256::digest(&bytes)).into(),
            bytes: bytes.into(),
            content_type: "application/x-tgsticker",
        })
    }
    pub fn parse(kind: MediaKind, bytes: Vec<u8>) -> Result<Self, MediaError> {
        match kind {
            MediaKind::Thumbnail => Self::thumbnail(bytes),
            MediaKind::Animation => Self::animation(bytes),
        }
    }
    pub fn kind(&self) -> MediaKind {
        self.kind
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn content_type(&self) -> &'static str {
        self.content_type
    }
    pub fn etag(&self) -> &str {
        &self.etag
    }
}

impl AsRef<[u8]> for MediaAsset {
    fn as_ref(&self) -> &[u8] {
        self.bytes()
    }
}
