//! Reuse content-addressed covers and bounded, coalesced Telegram photo uploads.
use super::{client::MtProtoActions, invocations::Group};
use crate::{BotError, Reply};
use grammers_client::{sender::InvocationError, tl};
use market_invocations::{Priority, Replay};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;
type Slot = Arc<tokio::sync::Mutex<Option<(Instant, tl::enums::InputPhoto)>>>;
#[derive(Default, Clone)]
pub(super) struct Photos(Arc<Mutex<VecDeque<(String, Slot)>>>);
impl Photos {
    fn slot(&self, id: &str) -> Slot {
        let mut slots = self.0.lock().unwrap();
        let slot = slots
            .iter()
            .position(|(key, _)| key == id)
            .and_then(|i| slots.remove(i))
            .map(|(_, slot)| slot)
            .unwrap_or_default();
        slots.push_back((id.to_owned(), slot.clone()));
        while slots.len() > 64 {
            slots.pop_front();
        }
        slot
    }
}
impl MtProtoActions {
    async fn photo(
        &self,
        cover: market_media::GiftCover,
    ) -> Result<tl::enums::InputPhoto, BotError> {
        let cache = self.artwork.as_ref().ok_or(BotError::Transport)?;
        let asset = match cache.cover(cover).await {
            Ok(asset) => asset,
            Err(error) => {
                tracing::warn!(
                    model_document_id = cover.model.get(),
                    ?error,
                    "Gift cover unavailable; checking thumbnail fallback"
                );
                fallback_asset(cache, cover.model, error).await?
            }
        };
        // Cache by image content, so backdrop/symbol changes cannot reuse the wrong photo.
        let slot = self.photos.slot(asset.etag());
        let mut cached = slot.lock().await;
        if let Some((until, photo)) = &*cached
            && *until > Instant::now()
        {
            return Ok(photo.clone());
        }
        let id = cover.model.get();
        // Telegram photo uploads reject WebP even though thumbnail delivery accepts it.
        let asset = if asset.content_type() == "image/webp" {
            tokio::task::spawn_blocking(move || photo_asset(asset))
                .await
                .map_err(|_| BotError::Transport)??
        } else {
            asset
        };
        let extension = match asset.content_type() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            _ => return Err(BotError::InvalidResponse),
        };
        let client = self.client.clone();
        let name = format!("gift.{extension}");
        let uploaded = self
            .invocations
            .run(
                Group::Media,
                Priority::Interactive,
                Replay::Safe,
                move || {
                    let client = client.clone();
                    let asset = asset.clone();
                    let name = name.clone();
                    async move {
                        client
                            .upload_stream(
                                &mut std::io::Cursor::new(asset.bytes()),
                                asset.bytes().len(),
                                name,
                            )
                            .await
                            .map_err(InvocationError::Io)
                    }
                },
            )
            .await
            .map_err(|error| error.bot())?;
        let request = tl::functions::messages::UploadMedia {
            business_connection_id: None,
            peer: tl::enums::InputPeer::PeerSelf,
            media: tl::types::InputMediaUploadedPhoto {
                spoiler: false,
                live_photo: false,
                file: uploaded.raw,
                stickers: None,
                ttl_seconds: None,
                video: None,
            }
            .into(),
        };
        let client = self.client.clone();
        let media = self
            .invocations
            .run(
                Group::Media,
                Priority::Interactive,
                Replay::Safe,
                move || {
                    let client = client.clone();
                    let request = request.clone();
                    async move { client.invoke(&request).await }
                },
            )
            .await
            .map_err(|error| {
                if let super::invocations::CallError::Remote(InvocationError::Rpc(rpc)) = &error {
                    #[cfg(test)]
                    eprintln!(
                        "Artwork upload RPC rejected: {} ({})",
                        super::invocations::safe_rpc_name(&rpc.name),
                        rpc.code
                    );
                    tracing::warn!(
                        stage = "photo_upload",
                        document_id = id,
                        rpc_code = rpc.code,
                        rpc_name = super::invocations::safe_rpc_name(&rpc.name),
                        "Telegram bot photo upload failed"
                    );
                }
                error.bot()
            })?;
        let tl::enums::MessageMedia::Photo(media) = media else {
            return Err(BotError::InvalidResponse);
        };
        let Some(tl::enums::Photo::Photo(photo)) = media.photo else {
            return Err(BotError::InvalidResponse);
        };
        let photo: tl::enums::InputPhoto = tl::types::InputPhoto {
            id: photo.id,
            access_hash: photo.access_hash,
            file_reference: photo.file_reference,
        }
        .into();
        *cached = Some((Instant::now() + Duration::from_secs(300), photo.clone()));
        Ok(photo)
    }
    pub(super) async fn prepare_rich(
        &self,
        reply: Option<&Reply>,
    ) -> Option<tl::enums::InputRichMessage> {
        let reply = reply?;
        let fallback = self.prepare_emojis(reply).await;
        if fallback.is_none() {
            return None;
        }
        let Some(id) = reply.artwork.filter(|_| {
            self.artwork.is_some()
                && reply
                    .rich_html
                    .as_ref()
                    .is_some_and(|html| html.len() <= 32700)
        }) else {
            return fallback;
        };
        match tokio::time::timeout(Duration::from_secs(6), self.photo(id)).await {
            Ok(Ok(photo)) => {
                let Some(tl::enums::InputRichMessage::Html(html)) = &fallback else {
                    return fallback;
                };
                Some(rich_with_photo(&html.html, photo))
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    document_id = id.model.get(),
                    ?error,
                    "Bot artwork preparation failed; sending valuation without image"
                );
                fallback
            }
            Err(_) => {
                tracing::warn!(
                    document_id = id.model.get(),
                    timeout_seconds = 6,
                    "Bot artwork preparation timed out; sending valuation without image"
                );
                fallback
            }
        }
    }
}

async fn fallback_asset(
    cache: &market_media::MediaCache,
    document: market_media::DocumentId,
    error: market_media::MediaError,
) -> Result<market_media::MediaAsset, BotError> {
    use market_media::{MediaError, MediaKey};
    let key = MediaKey::thumbnail(document);
    let result = match error {
        MediaError::NotFound | MediaError::InvalidMedia => cache.get(key).await,
        // Transient failures must not launch another download or bypass a server cooldown.
        _ => cache.cached(key).await.and_then(|asset| asset.ok_or(error)),
    };
    result.map_err(|error| match error {
        MediaError::RetryAfter(seconds) => BotError::RetryAfter(seconds),
        _ => BotError::Transport,
    })
}

fn rich_with_photo(html: &str, photo: tl::enums::InputPhoto) -> tl::enums::InputRichMessage {
    tl::types::InputRichMessageHtml {
        rtl: false,
        noautolink: true,
        html: format!("<img src=\"tg://photo?id=gift\"/>{html}"),
        files: Some(vec![
            tl::types::InputRichFilePhoto {
                id: "gift".into(),
                photo,
            }
            .into(),
        ]),
    }
    .into()
}

fn photo_asset(asset: market_media::MediaAsset) -> Result<market_media::MediaAsset, BotError> {
    if asset.content_type() != "image/webp" {
        return Ok(asset);
    }
    let mut reader = image::ImageReader::with_format(
        std::io::Cursor::new(asset.bytes()),
        image::ImageFormat::WebP,
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| BotError::InvalidResponse)?;
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|_| BotError::InvalidResponse)?;
    market_media::MediaAsset::thumbnail(output.into_inner()).map_err(|_| BotError::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn transient_cover_failures_reuse_only_cached_thumbnails_and_keep_cooldowns() {
        use market_media::{DocumentId, MediaCache, MediaError, MediaKey};
        let dir = tempfile::tempdir().unwrap();
        let (cache, mut downloads) = MediaCache::open(dir.path()).await.unwrap();
        let id = DocumentId::new(11).unwrap();
        assert_eq!(
            fallback_asset(&cache, id, MediaError::RetryAfter(17))
                .await
                .unwrap_err(),
            BotError::RetryAfter(17)
        );
        assert_eq!(
            fallback_asset(&cache, id, MediaError::Busy)
                .await
                .unwrap_err(),
            BotError::Transport
        );
        assert_eq!(cache.stats().queued, 0);
        let asset = photo_asset(webp(2, 2)).unwrap();
        let expected = asset.etag().to_owned();
        let worker = tokio::spawn(async move {
            downloads
                .run(|_| {
                    let asset = asset.clone();
                    async move { Ok(asset) }
                })
                .await;
        });
        cache.get(MediaKey::thumbnail(id)).await.unwrap();
        let requests = cache.stats().downloads;
        for error in [
            MediaError::RetryAfter(17),
            MediaError::Busy,
            MediaError::Unavailable,
        ] {
            assert_eq!(
                fallback_asset(&cache, id, error).await.unwrap().etag(),
                expected
            );
        }
        assert_eq!(cache.stats().downloads, requests);
        cache.shutdown().await;
        worker.await.unwrap();
    }
    fn webp(width: u32, height: u32) -> market_media::MediaAsset {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([31, 63, 127, 128]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::WebP)
            .unwrap();
        market_media::MediaAsset::thumbnail(bytes.into_inner()).unwrap()
    }
    #[test]
    fn webp_photo_conversion_preserves_pixels_and_bounds_decoding() {
        let png = photo_asset(webp(2, 2)).unwrap();
        assert_eq!(png.content_type(), "image/png");
        let decoded = image::load_from_memory_with_format(png.bytes(), image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.get_pixel(0, 0).0, [31, 63, 127, 128]);
        assert_eq!(photo_asset(png.clone()).unwrap().etag(), png.etag());
        assert!(photo_asset(webp(1025, 1)).is_err());
        let invalid =
            market_media::MediaAsset::thumbnail(b"RIFF\0\0\0\0WEBPbroken".to_vec()).unwrap();
        assert!(photo_asset(invalid).is_err());
    }
    #[test]
    fn rich_artwork_contains_exactly_one_uploaded_photo_and_keeps_body() {
        let photo = tl::types::InputPhoto {
            id: 1,
            access_hash: 2,
            file_reference: vec![],
        }
        .into();
        let tl::enums::InputRichMessage::Html(rich) =
            rich_with_photo("<p>Estimate</p><table compact></table>", photo)
        else {
            panic!()
        };
        assert_eq!(rich.html.matches("<img").count(), 1);
        assert!(
            rich.html
                .ends_with("<p>Estimate</p><table compact></table>")
        );
        let files = rich.files.unwrap();
        assert_eq!(files.len(), 1);
        let tl::enums::InputRichFile::Photo(file) = &files[0] else {
            panic!()
        };
        assert_eq!(file.id, "gift");
    }
    #[test]
    fn photo_upload_slots_are_shared_and_bounded() {
        let photos = Photos::default();
        let first = photos.slot("one");
        assert!(Arc::ptr_eq(&first, &photos.slot("one")));
        for id in 2..=100 {
            photos.slot(&id.to_string());
        }
        assert_eq!(photos.0.lock().unwrap().len(), 64);
        assert!(!Arc::ptr_eq(&first, &photos.slot("one")));
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::{
        BotCredentials, Connection,
        transport::{TelegramActions, Transport},
    };
    #[tokio::test]
    #[ignore = "requires explicit bot credentials and an artwork document ID; uploads a test photo"]
    async fn inspect_live_bot_artwork_upload() {
        let credentials = BotCredentials::new(
            std::env::var("TELEGRAM_BOT_TOKEN").unwrap(),
            std::env::var("TELEGRAM_API_ID").unwrap().parse().unwrap(),
            std::env::var("TELEGRAM_API_HASH").unwrap(),
        )
        .unwrap();
        assert_eq!(
            credentials.bot_id(),
            std::env::var("MARKET_TEST_BOT_ID")
                .unwrap()
                .parse::<u64>()
                .unwrap()
        );
        let dir = tempfile::tempdir().unwrap();
        let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
        let session_directory = std::env::var_os("MARKET_TEST_SESSION_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| dir.path().to_owned());
        let mut connection = Connection::open(credentials, &session_directory, scheduler.handle())
            .await
            .unwrap();
        let (cache, mut downloader) = market_media::MediaCache::open(dir.path().join("media"))
            .await
            .unwrap();
        connection.transport.bind_artwork(cache.clone());
        let actions = connection.transport.actions();
        actions.identity().await.unwrap();
        let media = actions.clone();
        let downloads = tokio::spawn(async move {
            downloader.run(|key| media.media(key)).await;
        });
        let id = std::env::var("MARKET_TEST_DOCUMENT_ID")
            .unwrap()
            .parse::<i64>()
            .unwrap();
        let started = Instant::now();
        let cover = market_media::GiftCover {
            model: market_media::DocumentId::new(id).unwrap(),
            pattern: None,
            palette: None,
        };
        let result = actions.photo(cover).await;
        eprintln!(
            "Artwork preparation: {} ms, result {:?}",
            started.elapsed().as_millis(),
            result.as_ref().map(|_| "photo")
        );
        cache.shutdown().await;
        let _ = downloads.await;
        connection.close().await.unwrap();
        scheduler.shutdown().await;
        result.unwrap();
    }
}
