//! Optional HTTP provider for API-only nodes. Only actor-validated catalog routes
//! are registered; redirects and proxy chains never forward arbitrary URLs.
use market_media::{MediaAsset, MediaError, MediaKey, MediaResult};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use url::Url;
pub(crate) const PROXY_HEADER: &str = "x-market-media-proxy";
#[derive(Clone)]
pub(crate) struct MediaOrigin {
    base: Url,
    client: reqwest::Client,
    routes: Arc<Mutex<HashMap<MediaKey, (String, std::time::Instant)>>>,
}
impl MediaOrigin {
    pub fn new(origin: &str) -> anyhow::Result<Self> {
        let base = Url::parse(origin)?;
        anyhow::ensure!(
            matches!(base.scheme(), "http" | "https")
                && base.host_str().is_some()
                && base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none()
                && base.path() == "/",
            "Media origin must be an HTTP(S) origin without a path or credentials"
        );
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(12))
            .no_proxy()
            .build()?;
        Ok(Self {
            base,
            client,
            routes: Arc::default(),
        })
    }
    pub fn register(&self, key: MediaKey, path: String) {
        let mut routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        if routes.len() >= 4096
            && !routes.contains_key(&key)
            && let Some(old) = routes
                .iter()
                .min_by_key(|(_, entry)| entry.1)
                .map(|(key, _)| *key)
        {
            routes.remove(&old);
        }
        routes.insert(key, (path, std::time::Instant::now()));
    }
    pub async fn fetch(&self, key: MediaKey) -> MediaResult {
        let path = self
            .routes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .map(|entry| entry.0.clone())
            .ok_or(MediaError::Unavailable)?;
        let url = self.base.join(&path).map_err(|_| MediaError::Unavailable)?;
        let mut response = self
            .client
            .get(url)
            .header(PROXY_HEADER, "1")
            .send()
            .await
            .map_err(|_| MediaError::Unavailable)?;
        match response.status().as_u16() {
            200 => {}
            404 => return Err(MediaError::NotFound),
            429 | 503 => {
                return Err(MediaError::RetryAfter(
                    response
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok())
                        .unwrap_or(2)
                        .clamp(1, 3600),
                ));
            }
            _ => return Err(MediaError::Unavailable),
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        if response.headers().contains_key("content-encoding")
            || response
                .content_length()
                .is_some_and(|n| n > key.max_bytes() as u64)
        {
            return Err(MediaError::InvalidMedia);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| MediaError::Unavailable)?
        {
            if bytes.len().saturating_add(chunk.len()) > key.max_bytes() {
                return Err(MediaError::InvalidMedia);
            }
            bytes.extend_from_slice(&chunk);
        }
        let asset = tokio::task::spawn_blocking(move || MediaAsset::parse(key.kind(), bytes))
            .await
            .map_err(|_| MediaError::Unavailable)??;
        if content_type != asset.content_type() {
            return Err(MediaError::InvalidMedia);
        }
        Ok(asset)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_is_an_explicit_root_without_credentials_or_url_tricks() {
        for valid in ["http://127.0.0.1:8081", "https://media.example/"] {
            assert!(MediaOrigin::new(valid).is_ok());
        }
        for invalid in [
            "file:///tmp/",
            "https://user:secret@example.com",
            "https://example.com/media",
            "https://example.com/?url=x",
            "https://example.com/#fragment",
        ] {
            assert!(MediaOrigin::new(invalid).is_err());
        }
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn origin(response: &'static [u8]) -> (MediaOrigin, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin =
            MediaOrigin::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let count = stream.read(&mut request).await.unwrap();
            stream.write_all(response).await.unwrap();
            String::from_utf8_lossy(&request[..count]).into_owned()
        });
        (origin, task)
    }
    #[tokio::test]
    async fn proxy_downloads_share_native_cache_and_survive_origin_failure() {
        let (origin,request) = origin(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 11\r\nConnection: close\r\n\r\n\x89PNG\r\n\x1a\nabc").await;
        let key = MediaKey::thumbnail(market_media::DocumentId::new(42).unwrap());
        origin.register(key, "/media/models/model".into());
        let directory = tempfile::tempdir().unwrap();
        let (cache, mut downloads) = market_media::MediaCache::open(directory.path())
            .await
            .unwrap();
        let worker = tokio::spawn(async move { downloads.run(|key| origin.fetch(key)).await });
        let (a, b) = tokio::join!(cache.get(key), cache.get(key));
        assert_eq!(a.unwrap().etag(), b.unwrap().etag());
        let request = request.await.unwrap();
        assert!(request.starts_with("GET /media/models/model HTTP/1.1"));
        assert!(request.contains("x-market-media-proxy: 1"));
        assert_eq!(cache.stats().downloads, 1);
        assert!(cache.get(key).await.is_ok());
        cache.shutdown().await;
        worker.await.unwrap();
        let (cache, downloader) = market_media::MediaCache::open(directory.path())
            .await
            .unwrap();
        drop(downloader);
        assert!(cache.get(key).await.is_ok());
    }
    #[tokio::test]
    async fn redirects_flood_waits_invalid_types_and_oversize_are_bounded() {
        let key = MediaKey::thumbnail(market_media::DocumentId::new(42).unwrap());
        for (response,expected) in [
            (&b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\n\r\n"[..], MediaError::Unavailable),
            (&b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 7\r\nContent-Length: 0\r\n\r\n"[..], MediaError::RetryAfter(7)),
            (&b"HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: 11\r\n\r\n\x89PNG\r\n\x1a\nabc"[..], MediaError::InvalidMedia),
            (&b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 262145\r\n\r\n"[..], MediaError::InvalidMedia),
            (&b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nbad\r\n0\r\n\r\n"[..], MediaError::InvalidMedia),
        ] {
            let (origin,request) = origin(response).await;
            origin.register(key,"/media/models/model".into());
            assert_eq!(origin.fetch(key).await.unwrap_err(), expected);
            request.await.unwrap();
        }
    }
}
