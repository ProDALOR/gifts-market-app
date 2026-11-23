use super::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
const CAPACITY: usize = 64;
fn image() -> MediaAsset {
    MediaAsset::thumbnail(b"\x89PNG\r\n\x1a\nsynthetic-test-image".to_vec()).unwrap()
}
fn key(id: i64) -> MediaKey {
    MediaKey::thumbnail(DocumentId::new(id).unwrap())
}
#[tokio::test]
async fn concurrent_misses_download_once_and_survive_restart_without_a_provider() {
    let directory = tempfile::tempdir().unwrap();
    let (cache, mut downloads) = MediaCache::open(directory.path()).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let release = Arc::new(tokio::sync::Notify::new());
    let gate = release.clone();
    let worker = tokio::spawn(async move {
        downloads
            .run(|_| async {
                counted.fetch_add(1, Ordering::SeqCst);
                gate.notified().await;
                Ok(image())
            })
            .await
    });
    let first_cache = cache.clone();
    let first = tokio::spawn(async move { first_cache.get(key(42)).await.unwrap() });
    let second_cache = cache.clone();
    let second = tokio::spawn(async move { second_cache.get(key(42)).await.unwrap() });
    tokio::time::timeout(Duration::from_secs(2), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    release.notify_one();
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.unwrap().etag(), second.unwrap().etag());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    worker.abort();
    let _ = worker.await;
    drop(cache);
    let (cache, downloads) = MediaCache::open(directory.path()).await.unwrap();
    drop(downloads);
    assert_eq!(cache.get(key(42)).await.unwrap().bytes(), image().bytes());
    assert!(matches!(
        cache.get(key(99)).await,
        Err(MediaError::Unavailable)
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
#[tokio::test]
async fn unavailable_models_are_negatively_cached_and_untrusted_payloads_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let (cache, mut downloads) = MediaCache::open(directory.path()).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let worker = tokio::spawn(async move {
        downloads
            .run(|_| async {
                counted.fetch_add(1, Ordering::SeqCst);
                Err(MediaError::NotFound)
            })
            .await
    });
    for _ in 0..2 {
        assert!(matches!(
            cache.get(key(42)).await,
            Err(MediaError::NotFound)
        ));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(cache.cached(key(42)).await.unwrap().is_none());
    worker.abort();
    let _ = worker.await;
    assert!(MediaAsset::thumbnail(b"<svg onload='anything'>".to_vec()).is_err());
    assert!(MediaAsset::thumbnail(vec![0; MAX_THUMBNAIL_BYTES + 1]).is_err());
    assert!(DocumentId::new(0).is_none());
    assert!(DocumentId::new(-1).is_none());
}
#[tokio::test]
async fn cancelled_transport_releases_inflight_work_and_can_resume_the_queue() {
    let directory = tempfile::tempdir().unwrap();
    let (cache, mut downloads) = MediaCache::open(directory.path()).await.unwrap();
    let reader = cache.clone();
    let request = tokio::spawn(async move { reader.get(key(42)).await });
    let started = tokio::sync::Notify::new();
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::select! {
            _ = started.notified() => {},
            _ = downloads.run(|_| async {
                started.notify_one();
                std::future::pending::<MediaResult>().await
            }) => panic!("download unexpectedly finished"),
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        request.await.unwrap(),
        Err(MediaError::Unavailable)
    ));
    let worker = tokio::spawn(async move { downloads.run(|_| async { Ok(image()) }).await });
    assert!(cache.get(key(42)).await.is_ok());
    worker.abort();
    let _ = worker.await;
}

#[tokio::test]
async fn queue_is_bounded_and_shutdown_wakes_waiting_readers() {
    let directory = tempfile::tempdir().unwrap();
    let (cache, downloads) = MediaCache::open(directory.path()).await.unwrap();
    let mut readers = Vec::new();
    for id in 1..=CAPACITY {
        let cache = cache.clone();
        readers.push(tokio::spawn(async move { cache.get(key(id as i64)).await }));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while cache.stats().queued < CAPACITY {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(cache.get(key(9999)).await, Err(MediaError::Busy)));
    drop(downloads);
    for reader in readers {
        assert!(matches!(
            reader.await.unwrap(),
            Err(MediaError::Unavailable)
        ));
    }
}

fn tgs_bytes(json: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(json).unwrap();
    encoder.finish().unwrap()
}
fn animation() -> MediaAsset {
    MediaAsset::animation(tgs_bytes(
        br#"{"v":"5.7.4","w":512,"h":512,"fr":60,"ip":0,"op":180,"layers":[],"assets":[]}"#,
    ))
    .unwrap()
}
#[tokio::test]
async fn animation_and_thumbnail_have_separate_cache_entries_and_survive_offline_restart() {
    let directory = tempfile::tempdir().unwrap();
    let (cache, mut downloads) = MediaCache::open(directory.path()).await.unwrap();
    let worker = tokio::spawn(async move {
        downloads
            .run(|key| async move {
                Ok(match key.kind() {
                    MediaKind::Thumbnail => image(),
                    MediaKind::Animation => animation(),
                    MediaKind::Preview | MediaKind::Cover => {
                        unreachable!("Previews are generated locally")
                    }
                })
            })
            .await
    });
    let (thumbnail, animation) = tokio::join!(
        cache.get(key(42)),
        cache.get(MediaKey::animation(key(42).document()))
    );
    let (thumbnail, animation) = (thumbnail.unwrap(), animation.unwrap());
    assert_eq!(thumbnail.content_type(), "image/png");
    assert_eq!(animation.content_type(), "application/x-tgsticker");
    assert_ne!(thumbnail.etag(), animation.etag());
    assert!(directory.path().join("42.thumb").is_file());
    assert!(directory.path().join("42.tgs").is_file());
    worker.abort();
    let _ = worker.await;
    let (cache, downloads) = MediaCache::open(directory.path()).await.unwrap();
    drop(downloads);
    assert_eq!(
        cache
            .get(MediaKey::animation(key(42).document()))
            .await
            .unwrap()
            .bytes(),
        animation.bytes()
    );
    assert_eq!(cache.get(key(42)).await.unwrap().bytes(), thumbnail.bytes());
}
#[test]
fn animations_reject_invalid_gzip_expansion_bombs_external_assets_and_expressions() {
    assert!(MediaAsset::animation(b"not gzip".to_vec()).is_err());
    assert!(MediaAsset::animation(tgs_bytes(b"not JSON")).is_err());
    assert!(MediaAsset::animation(tgs_bytes(&vec![b' '; MAX_TGS_JSON_BYTES + 1])).is_err());
    let base = serde_json::json!({"v":"5.7.4","w":512,"h":512,"fr":60,"ip":0,"op":180,"layers":[],"assets":[]});
    for patch in [
        serde_json::json!({"w":1000000}),
        serde_json::json!({"fr":0}),
        serde_json::json!({"op":360000}),
        serde_json::json!({"assets":[{"p":"https://example.com/file.png"}]}),
        serde_json::json!({"layers":[{"x":"an expression"}]}),
    ] {
        let mut data = base.clone();
        data.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(MediaAsset::animation(tgs_bytes(&serde_json::to_vec(&data).unwrap())).is_err());
    }
    assert_eq!(animation().kind(), MediaKind::Animation);
}

async fn wait_for(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !ready() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn hot_hits_skip_disk_and_memory_evicts_by_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let limits = MediaLimits {
        memory_bytes: image().bytes().len(),
        ..MediaLimits::default()
    };
    let (cache, downloads) = MediaCache::with_limits(directory.path(), limits)
        .await
        .unwrap();
    drop(downloads);
    for id in [1, 2] {
        std::fs::write(
            directory.path().join(format!("{id}.thumb")),
            image().bytes(),
        )
        .unwrap();
    }
    cache.get(key(1)).await.unwrap();
    for _ in 0..10 {
        cache.get(key(1)).await.unwrap();
    }
    assert_eq!(cache.stats().disk_reads, 1);
    assert_eq!(cache.stats().memory_hits, 10);
    cache.get(key(2)).await.unwrap();
    assert_eq!(cache.stats().memory_bytes, image().bytes().len());
    cache.get(key(1)).await.unwrap();
    assert_eq!(cache.stats().disk_reads, 3);
}
#[tokio::test]
async fn coalesced_disk_reads_and_ttl_refresh_external_changes() {
    let directory = tempfile::tempdir().unwrap();
    let limits = MediaLimits {
        memory_ttl: Duration::from_millis(50),
        ..MediaLimits::default()
    };
    std::fs::write(directory.path().join("1.thumb"), image().bytes()).unwrap();
    let (cache, downloads) = MediaCache::with_limits(directory.path(), limits)
        .await
        .unwrap();
    drop(downloads);
    let futures = (0..20).map(|_| cache.get(key(1)));
    for result in futures_util::future::join_all(futures).await {
        result.unwrap();
    }
    assert_eq!(cache.stats().disk_reads, 1);
    std::fs::write(directory.path().join("1.thumb"), b"invalid").unwrap();
    assert!(cache.get(key(1)).await.is_ok());
    tokio::time::sleep(Duration::from_millis(70)).await;
    assert_eq!(
        cache.get(key(1)).await.unwrap_err(),
        MediaError::Unavailable
    );
    assert!(!directory.path().join("1.thumb").exists());
}
#[tokio::test]
async fn disk_quota_and_stale_temporary_cleanup_preserve_unrelated_files() {
    let directory = tempfile::tempdir().unwrap();
    let stale = directory
        .path()
        .join(format!("1.{}.part", uuid::Uuid::new_v4()));
    std::fs::write(&stale, b"incomplete").unwrap();
    std::fs::File::open(&stale)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH))
        .unwrap();
    std::fs::write(directory.path().join("keep.part"), b"unrelated").unwrap();
    let recent = directory
        .path()
        .join(format!("2.{}.part", uuid::Uuid::new_v4()));
    std::fs::write(&recent, b"still writing").unwrap();
    let limits = MediaLimits {
        disk_bytes: image().bytes().len() as u64 * 2,
        ..MediaLimits::default()
    };
    let (cache, downloads) = MediaCache::with_limits(directory.path(), limits)
        .await
        .unwrap();
    drop(downloads);
    assert!(!stale.exists());
    assert!(recent.exists());
    for id in 1..=4 {
        cache.store(key(id), image()).await.unwrap();
    }
    assert!(!directory.path().join("1.thumb").exists());
    assert!(!directory.path().join("2.thumb").exists());
    assert!(directory.path().join("3.thumb").exists());
    assert!(directory.path().join("4.thumb").exists());
    assert!(directory.path().join("keep.part").exists());
    cache.store(key(4), image()).await.unwrap();
    assert!(directory.path().join("3.thumb").exists());
    cache.shutdown().await;
    assert_eq!(
        cache.get(key(4)).await.unwrap_err(),
        MediaError::Unavailable
    );
}
#[tokio::test]
async fn downloads_are_fifo_and_abandoned_requests_are_skipped() {
    let directory = tempfile::tempdir().unwrap();
    let (cache, mut downloads) = MediaCache::open(directory.path()).await.unwrap();
    let first_cache = cache.clone();
    let first = tokio::spawn(async move { first_cache.get(key(1)).await });
    wait_for(|| cache.stats().queued == 1).await;
    let cancelled_cache = cache.clone();
    let cancelled = tokio::spawn(async move { cancelled_cache.get(key(2)).await });
    wait_for(|| cache.stats().queued == 2).await;
    cancelled.abort();
    let _ = cancelled.await;
    let last_cache = cache.clone();
    let last = tokio::spawn(async move { last_cache.get(key(3)).await });
    wait_for(|| cache.stats().queued == 3).await;
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = order.clone();
    let worker = tokio::spawn(async move {
        downloads
            .run(|key| {
                observed.lock().unwrap().push(key);
                async { Ok(image()) }
            })
            .await
    });
    last.await.unwrap().unwrap();
    first.await.unwrap().unwrap();
    wait_for(|| cache.stats().skipped_downloads == 1).await;
    assert_eq!(*order.lock().unwrap(), [key(1), key(3)]);
    cache.shutdown().await;
    worker.await.unwrap();
}
#[tokio::test]
async fn downloads_run_concurrently_with_a_hard_limit_and_timeout() {
    let directory = tempfile::tempdir().unwrap();
    let limits = MediaLimits {
        downloads: 2,
        start_interval: Duration::ZERO,
        download_timeout: Duration::from_millis(100),
        request_timeout: Duration::from_secs(2),
        ..MediaLimits::default()
    };
    let (cache, mut downloads) = MediaCache::with_limits(directory.path(), limits)
        .await
        .unwrap();
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (r, p) = (running.clone(), peak.clone());
    struct Count(Arc<AtomicUsize>);
    impl Drop for Count {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let worker = tokio::spawn(async move {
        downloads
            .run(|_| async {
                let _count = Count(r.clone());
                p.fetch_max(r.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                std::future::pending::<MediaResult>().await
            })
            .await
    });
    let futures = (1..=4).map(|id| cache.get(key(id)));
    for result in futures_util::future::join_all(futures).await {
        assert_eq!(result.unwrap_err(), MediaError::Unavailable);
    }
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    assert_eq!(running.load(Ordering::SeqCst), 0);
    cache.shutdown().await;
    worker.await.unwrap();
}

#[tokio::test]
async fn shutdown_wakes_active_readers_without_waiting_for_transport_timeout() {
    let directory = tempfile::tempdir().unwrap();
    let (cache, mut downloads) = MediaCache::open(directory.path()).await.unwrap();
    let worker = tokio::spawn(async move {
        downloads
            .run(|_| std::future::pending::<MediaResult>())
            .await
    });
    let reader = cache.clone();
    let request = tokio::spawn(async move { reader.get(key(42)).await });
    wait_for(|| cache.stats().downloads == 1).await;
    cache.shutdown().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), request)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err(),
        MediaError::Unavailable
    );
    worker.abort();
    let _ = worker.await;
}
