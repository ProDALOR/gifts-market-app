use sha2::{Digest, Sha256};
use std::sync::Arc;
pub const MAX_ANIMATION_BYTES: usize = 256 * 1024;
pub const MAX_TGS_JSON_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_THUMBNAIL_BYTES: usize = 256 * 1024;
pub const MAX_PREVIEW_BYTES: usize = 1024 * 1024;
pub const PREVIEW_VERSION: &str = "2";
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
    Preview,
}
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct MediaKey {
    document: DocumentId,
    kind: MediaKind,
    source: Option<[u8; 32]>,
}
impl MediaKey {
    pub fn thumbnail(document: DocumentId) -> Self {
        Self {
            document,
            kind: MediaKind::Thumbnail,
            source: None,
        }
    }
    pub fn animation(document: DocumentId) -> Self {
        Self {
            document,
            kind: MediaKind::Animation,
            source: None,
        }
    }
    pub(crate) fn preview(document: DocumentId, source: [u8; 32]) -> Self {
        Self {
            document,
            kind: MediaKind::Preview,
            source: Some(source),
        }
    }
    pub fn document(self) -> DocumentId {
        self.document
    }
    pub fn kind(self) -> MediaKind {
        self.kind
    }
    pub(crate) fn source(self) -> Option<[u8; 32]> {
        self.source
    }
    pub fn max_bytes(self) -> usize {
        match self.kind {
            MediaKind::Thumbnail => MAX_THUMBNAIL_BYTES,
            MediaKind::Animation => MAX_ANIMATION_BYTES,
            MediaKind::Preview => MAX_PREVIEW_BYTES,
        }
    }
    pub(crate) fn extension(self) -> String {
        match self.kind {
            MediaKind::Thumbnail => "thumb".into(),
            MediaKind::Animation => "tgs".into(),
            MediaKind::Preview => {
                let hash = self
                    .source
                    .expect("derived previews require source identity");
                let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
                format!("preview-{PREVIEW_VERSION}-{hex}.svg")
            }
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
    pub(crate) digest: [u8; 32],
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
        Ok(Self::new(MediaKind::Thumbnail, bytes, content_type))
    }
    pub fn animation(bytes: Vec<u8>) -> Result<Self, MediaError> {
        let bytes = crate::tgs::normalize(bytes)?;
        Ok(Self::new(
            MediaKind::Animation,
            bytes,
            "application/x-tgsticker",
        ))
    }
    pub(crate) fn preview(bytes: Vec<u8>) -> Result<Self, MediaError> {
        crate::preview::validate(&bytes)?;
        Ok(Self::new(MediaKind::Preview, bytes, "image/svg+xml"))
    }
    fn new(kind: MediaKind, bytes: Vec<u8>, content_type: &'static str) -> Self {
        let digest = Sha256::digest(&bytes);
        // SVG can be compressed by HTTP middleware; weak tags identify equivalent representations.
        let prefix = if content_type == "image/svg+xml" {
            "W/"
        } else {
            ""
        };
        Self {
            kind,
            etag: format!("{prefix}\"{digest:x}\"").into(),
            digest: digest.into(),
            bytes: bytes.into(),
            content_type,
        }
    }
    pub fn parse(kind: MediaKind, bytes: Vec<u8>) -> Result<Self, MediaError> {
        match kind {
            MediaKind::Thumbnail => Self::thumbnail(bytes),
            MediaKind::Animation => Self::animation(bytes),
            MediaKind::Preview => Self::preview(bytes),
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
