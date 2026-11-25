//! Resolve fresh documents and download thumbnails or TGS.
use super::invocations::{CallError, Executor, Group};
use crate::BotError;
use grammers_client::{
    Client,
    media::{Document, PhotoSize},
    sender::InvocationError,
    tl,
};
use market_invocations::{Priority, Replay};
use market_media::{MAX_THUMBNAIL_BYTES, MediaAsset, MediaError, MediaKey, MediaKind};

#[tracing::instrument(skip_all, fields(document_id = key.document().get(), variant = ?key.kind()))]
pub(super) async fn get(
    client: &Client,
    invocations: &Executor,
    documents: &Documents,
    key: MediaKey,
) -> Result<MediaAsset, MediaError> {
    let id = key.document().get();
    for attempt in 0..2 {
        let entry = documents.entry(id);
        let raw = entry
            .value
            .get_or_init(|| async {
                invocations
                    .rpc(
                        client,
                        tl::functions::messages::GetCustomEmojiDocuments {
                            document_id: vec![id],
                        },
                        Group::Media,
                        Priority::Background,
                        Replay::Safe,
                    )
                    .await
                    .map_err(|error| {
                        tracing::warn!(
                            stage = "document_lookup",
                            ?error,
                            "Telegram media lookup failed"
                        );
                        failure(error)
                    })?
                    .into_iter()
                    .find(|document| document.id() == id)
                    .ok_or_else(|| {
                        tracing::warn!(
                            stage = "document_lookup",
                            "Telegram returned no matching media document"
                        );
                        MediaError::NotFound
                    })
            })
            .await
            .clone();
        let raw = match raw {
            Ok(raw) => raw,
            Err(error) => {
                documents.invalidate(id, &entry);
                return Err(error);
            }
        };
        let source = client.clone();
        // Restart a bounded asset on retry: DownloadIter consumes its request on errors.
        let result = invocations
            .run(
                Group::Media,
                Priority::Background,
                Replay::Safe,
                move || {
                    let source = source.clone();
                    let raw = raw.clone();
                    async move {
                        match download_bytes(&source, key, raw).await {
                            Ok(bytes) => Ok(Ok(bytes)),
                            Err(DownloadError::Media(e)) => Ok(Err(e)),
                            Err(DownloadError::Remote(e)) => Err(e),
                        }
                    }
                },
            )
            .await;
        match result {
            Ok(Ok(bytes)) => return asset(key, bytes).await,
            Ok(Err(error)) => return Err(error),
            Err(CallError::Remote(InvocationError::Rpc(error)))
                if matches!(
                    error.name.as_str(),
                    "FILE_REFERENCE_EXPIRED" | "FILE_REFERENCE_INVALID" | "FILE_REFERENCE_EMPTY"
                ) =>
            {
                documents.invalidate(id, &entry);
                if attempt == 1 {
                    return Err(MediaError::Unavailable);
                }
            }
            Err(error) => {
                match &error {
                    CallError::Remote(InvocationError::Rpc(error)) => {
                        tracing::warn!(stage = "download", rpc_code = error.code, rpc_name = %error.name, "Telegram media download failed")
                    }
                    _ => tracing::warn!(stage = "download", "Telegram media download interrupted"),
                }
                return Err(failure(error.bot()));
            }
        }
    }
    Err(MediaError::Unavailable)
}
#[derive(Debug)]
enum DownloadError {
    Remote(InvocationError),
    Media(MediaError),
}
impl From<MediaError> for DownloadError {
    fn from(error: MediaError) -> Self {
        Self::Media(error)
    }
}
#[tracing::instrument(skip_all, fields(document_id = key.document().get(), variant = ?key.kind()))]
async fn download_bytes(
    client: &Client,
    key: MediaKey,
    raw: tl::enums::Document,
) -> Result<Vec<u8>, DownloadError> {
    let media = Document::from_raw_media(tl::types::MessageMediaDocument {
        document: Some(raw),
        nopremium: false,
        spoiler: false,
        video: false,
        round: false,
        voice: false,
        alt_documents: None,
        video_cover: None,
        video_timestamp: None,
        ttl_seconds: None,
    });
    let mut download = match key.kind() {
        MediaKind::Preview | MediaKind::Cover => return Err(MediaError::InvalidMedia.into()),
        MediaKind::Thumbnail
            if matches!(
                media.mime_type(),
                Some("image/webp" | "image/png" | "image/jpeg")
            ) =>
        {
            if media
                .size()
                .is_none_or(|size| size == 0 || size > key.max_bytes())
            {
                return Err(MediaError::InvalidMedia.into());
            }
            client.iter_download(&media)
        }
        MediaKind::Thumbnail => {
            let thumb = media
                .thumbs()
                .into_iter()
                .filter(|thumb| {
                    matches!(
                        thumb,
                        PhotoSize::Size(_) | PhotoSize::Cached(_) | PhotoSize::Progressive(_)
                    )
                })
                .filter(|thumb| thumb.size() > 0 && thumb.size() <= MAX_THUMBNAIL_BYTES)
                .max_by_key(PhotoSize::size)
                .ok_or(MediaError::NotFound)?;
            client.iter_download(&thumb)
        }
        MediaKind::Animation => {
            if media.mime_type() != Some("application/x-tgsticker") {
                tracing::warn!(
                    stage = "download",
                    reason = "mime_type",
                    mime_type = media.mime_type(),
                    "Telegram animation is not TGS"
                );
                return Err(MediaError::NotFound.into());
            }
            if media
                .size()
                .is_none_or(|size| size == 0 || size > key.max_bytes())
            {
                tracing::warn!(
                    stage = "download",
                    reason = "declared_size",
                    bytes = media.size(),
                    limit = key.max_bytes(),
                    "Telegram animation rejected"
                );
                return Err(MediaError::InvalidMedia.into());
            }
            client.iter_download(&media)
        }
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = download.next().await.map_err(DownloadError::Remote)? {
        if bytes.len().saturating_add(chunk.len()) > key.max_bytes() {
            tracing::warn!(
                stage = "download",
                reason = "stream_size",
                limit = key.max_bytes(),
                "Telegram media exceeds size limit"
            );
            return Err(MediaError::InvalidMedia.into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn asset(key: MediaKey, bytes: Vec<u8>) -> Result<MediaAsset, MediaError> {
    match key.kind() {
        MediaKind::Preview | MediaKind::Cover => Err(MediaError::InvalidMedia),
        MediaKind::Thumbnail => MediaAsset::thumbnail(bytes),
        MediaKind::Animation => {
            let span = tracing::Span::current();
            tokio::task::spawn_blocking(move || span.in_scope(|| MediaAsset::animation(bytes)))
                .await
                .map_err(|_| MediaError::Unavailable)?
        }
    }
}
fn failure(error: BotError) -> MediaError {
    match error {
        BotError::RetryAfter(seconds) => MediaError::RetryAfter(seconds),
        BotError::Rpc(400 | 403 | 404) => MediaError::NotFound,
        _ => MediaError::Unavailable,
    }
}

// References stay private and short-lived; assets and inline emoji share metadata.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
struct Entry {
    created: Instant,
    value: tokio::sync::OnceCell<Result<tl::enums::Document, MediaError>>,
}
#[derive(Clone, Default)]
pub(super) struct Documents {
    entries: Arc<Mutex<HashMap<i64, Arc<Entry>>>>,
    batches: Arc<tokio::sync::Mutex<()>>,
}
impl Documents {
    /// Metadata-only batches share document entries with thumbnail/TGS downloads.
    pub(super) async fn emoji_alternatives<F, Fut>(
        &self,
        ids: &[market_gifts::TelegramDocumentId],
        fetch: F,
    ) -> Result<std::collections::BTreeMap<i64, String>, BotError>
    where
        F: FnOnce(Vec<i64>) -> Fut,
        Fut: std::future::Future<Output = Result<Vec<tl::enums::Document>, BotError>>,
    {
        let _batch = self.batches.lock().await;
        let entries: std::collections::BTreeMap<_, _> = ids
            .iter()
            .take(64)
            .map(|id| (id.get(), self.entry(id.get())))
            .collect();
        let missing = entries
            .iter()
            .filter(|(_, entry)| entry.value.get().is_none())
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            let mut found: HashMap<_, _> = fetch(missing.clone())
                .await?
                .into_iter()
                .map(|document| (document.id(), document))
                .collect();
            for id in missing {
                let _ = entries[&id]
                    .value
                    .set(found.remove(&id).ok_or(MediaError::NotFound));
            }
        }
        Ok(entries
            .iter()
            .filter_map(|(id, entry)| {
                let tl::enums::Document::Document(document) = entry.value.get()?.as_ref().ok()?
                else {
                    return None;
                };
                document
                    .attributes
                    .iter()
                    .find_map(|attribute| match attribute {
                        tl::enums::DocumentAttribute::CustomEmoji(emoji)
                            if !emoji.alt.is_empty() =>
                        {
                            Some((*id, emoji.alt.clone()))
                        }
                        _ => None,
                    })
            })
            .collect())
    }
    fn entry(&self, id: i64) -> Arc<Entry> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|_, entry| entry.created.elapsed() < Duration::from_secs(300));
        if entries.len() >= 128
            && !entries.contains_key(&id)
            && let Some(old) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.created)
                .map(|(id, _)| *id)
        {
            entries.remove(&old);
        }
        entries
            .entry(id)
            .or_insert_with(|| {
                Arc::new(Entry {
                    created: Instant::now(),
                    value: tokio::sync::OnceCell::new(),
                })
            })
            .clone()
    }
    fn invalidate(&self, id: i64, entry: &Arc<Entry>) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if entries
            .get(&id)
            .is_some_and(|current| Arc::ptr_eq(current, entry))
        {
            entries.remove(&id);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn emoji_batches_coalesce_and_cache_unusable_documents_without_downloading_files() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let documents = Documents::default();
        let id = market_gifts::TelegramDocumentId::new(7).unwrap();
        let count = AtomicUsize::new(0);
        let fetch = |ids: Vec<i64>| {
            let count = &count;
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                assert_eq!(ids, vec![7]);
                tokio::task::yield_now().await;
                Ok(vec![tl::types::DocumentEmpty { id: 7 }.into()])
            }
        };
        let ids = [id, id];
        let (a, b) = tokio::join!(
            documents.emoji_alternatives(&ids, fetch),
            documents.emoji_alternatives(&ids, fetch)
        );
        assert!(a.unwrap().is_empty());
        assert!(b.unwrap().is_empty());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn temporary_emoji_lookup_failure_does_not_poison_document_cache() {
        let documents = Documents::default();
        let ids = [market_gifts::TelegramDocumentId::new(7).unwrap()];
        assert!(
            documents
                .emoji_alternatives(&ids, |_| async { Err(BotError::Transport) })
                .await
                .is_err()
        );
        assert!(
            documents
                .emoji_alternatives(&ids, |_| async { Ok(vec![]) })
                .await
                .unwrap()
                .is_empty()
        );
    }
    #[tokio::test]
    async fn references_coalesce_and_an_old_failure_cannot_remove_a_refresh() {
        let documents = Documents::default();
        let (first, second) = (documents.entry(42), documents.entry(42));
        assert!(Arc::ptr_eq(&first, &second));
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let resolve = || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tokio::task::yield_now().await;
            Ok(tl::types::DocumentEmpty { id: 42 }.into())
        };
        let (a, b) = tokio::join!(
            first.value.get_or_init(resolve),
            second.value.get_or_init(resolve)
        );
        assert_eq!(a.as_ref().unwrap().id(), b.as_ref().unwrap().id());
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        documents.invalidate(42, &first);
        let fresh = documents.entry(42);
        assert!(!Arc::ptr_eq(&fresh, &first));
        documents.invalidate(42, &first);
        assert!(Arc::ptr_eq(&fresh, &documents.entry(42)));
        for id in 1..=1000 {
            documents.entry(id);
        }
        assert_eq!(documents.entries.lock().unwrap().len(), 128);
    }
}
