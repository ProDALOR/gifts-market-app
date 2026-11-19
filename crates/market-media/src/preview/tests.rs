use super::*;
use std::io::Write;
fn animation(name: &str) -> MediaAsset {
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    write!(gzip, "{{\"v\":\"5.7.4\",\"w\":512,\"h\":512,\"fr\":60,\"ip\":0,\"op\":180,\"nm\":\"{name}\",\"layers\":[]}}").unwrap();
    MediaAsset::animation(gzip.finish().unwrap()).unwrap()
}
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><path d="M0 0L1 1" fill="#ffffff"/></svg>"##;
#[test]
fn validation_rejects_active_external_and_oversized_svg() {
    assert!(validate(SVG.as_bytes()).is_ok());
    for bad in [
        SVG.replace("<path", "<path onload=\"alert(1)\""),
        SVG.replace("#ffffff", "url(https://example.test/image)"),
        SVG.replace("<path", "<foreignObject"),
        SVG.replace("<path", "<script"),
        SVG.replace("<path", "<image href=\"external\""),
        SVG.replace(
            "<path",
            "<path style=\"background:url(https://example.test)\"",
        ),
        format!("<!DOCTYPE svg [<!ENTITY x SYSTEM 'file:///etc/passwd'>]>{SVG}"),
    ] {
        assert!(validate(bad.as_bytes()).is_err(), "{bad}");
    }
    assert!(validate(&vec![b' '; MAX_PREVIEW_BYTES + 1]).is_err());
}
#[cfg(unix)]
fn helper(directory: &std::path::Path, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let file = directory.join("renderer");
    std::fs::write(
        &file,
        format!("#!/bin/sh\nprintf x >> \"$0.calls\"\n{body}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    file
}
#[cfg(unix)]
#[tokio::test]
async fn coalescing_source_invalidation_and_persistence() {
    let directory = tempfile::tempdir().unwrap();
    let renderer = helper(
        directory.path(),
        &format!("sleep 0.05\nprintf '%s' '{SVG}' > \"$2\""),
    );
    let path = directory.path().join("cache");
    let (cache, _downloads) = MediaCache::with_renderer(&path, Default::default(), renderer)
        .await
        .unwrap();
    let document = DocumentId::new(42).unwrap();
    cache
        .store(MediaKey::animation(document), animation("first"))
        .await
        .unwrap();
    for copy in futures_util::future::join_all((0..20).map(|_| cache.preview(document))).await {
        assert_eq!(copy.unwrap().bytes(), SVG.as_bytes());
    }
    assert_eq!(cache.stats().preview_generations, 1);
    assert_eq!(
        std::fs::read(directory.path().join("renderer.calls")).unwrap(),
        b"x"
    );
    cache
        .store(MediaKey::animation(document), animation("changed"))
        .await
        .unwrap();
    cache.preview(document).await.unwrap();
    assert_eq!(cache.stats().preview_generations, 2);
    assert_eq!(
        std::fs::read(directory.path().join("renderer.calls")).unwrap(),
        b"xx"
    );
    cache.shutdown().await;
    let (reopened, _downloads) = MediaCache::with_renderer(
        &path,
        Default::default(),
        directory.path().join("not-installed"),
    )
    .await
    .unwrap();
    assert_eq!(
        reopened.preview(document).await.unwrap().bytes(),
        SVG.as_bytes()
    );
    assert_eq!(reopened.stats().preview_generations, 0);
    assert_eq!(reopened.stats().downloads, 0);
    reopened.shutdown().await;
}
#[cfg(unix)]
#[tokio::test]
async fn abandonment_and_shutdown_kill_and_reap_the_child() {
    let directory = tempfile::tempdir().unwrap();
    let renderer = helper(directory.path(), "echo $$ > \"$0.pid\"\nexec sleep 30");
    let (cache, _downloads) =
        MediaCache::with_renderer(directory.path().join("cache"), Default::default(), renderer)
            .await
            .unwrap();
    let document = DocumentId::new(7).unwrap();
    cache
        .store(MediaKey::animation(document), animation("cancel"))
        .await
        .unwrap();
    let worker = cache.clone();
    let request = tokio::spawn(async move { worker.preview(document).await });
    let pid = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(pid) = tokio::fs::read_to_string(directory.path().join("renderer.pid")).await
            {
                break pid.trim().to_owned();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    request.abort();
    let _ = request.await;
    tokio::time::timeout(Duration::from_secs(2), cache.shutdown())
        .await
        .unwrap();
    assert!(
        !std::process::Command::new("kill")
            .args(["-0", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
}
#[cfg(unix)]
#[tokio::test]
async fn conversion_budget_and_shared_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let renderer = helper(
        directory.path(),
        &format!(
            "while [ ! -f \"$0.release\" ]; do sleep 0.02; done\nprintf '%s' '{SVG}' > \"$2\""
        ),
    );
    let (cache, _downloads) =
        MediaCache::with_renderer(directory.path().join("cache"), Default::default(), renderer)
            .await
            .unwrap();
    let mut jobs = Vec::new();
    for id in 1..=3 {
        let source = animation("budget");
        let document = DocumentId::new(id).unwrap();
        jobs.push(
            cache
                .job(MediaKey::preview(document, source.digest))
                .unwrap(),
        );
        cache
            .store(MediaKey::animation(document), source)
            .await
            .unwrap();
    }
    let mut requests = Vec::new();
    for id in [1, 1, 2, 3] {
        let cache = cache.clone();
        requests.push(tokio::spawn(async move {
            cache.preview(DocumentId::new(id).unwrap()).await
        }));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let admitted = jobs
                .iter()
                .filter(|job| job.queued.load(Ordering::Acquire))
                .count();
            // Observe admission, not OS shell startup latency. Both slots remain held
            // until the helper is released, and the third job must stay queued.
            if admitted == 3 && cache.stats().preview_generations == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(cache.0.preview_slots.available_permits(), 0);
    assert_eq!(cache.stats().preview_generations, 2);
    // Dropping one caller must not cancel another caller for the same source.
    let cancelled = requests.remove(0);
    cancelled.abort();
    let _ = cancelled.await;
    tokio::fs::write(directory.path().join("renderer.release"), b"")
        .await
        .unwrap();
    for request in requests {
        assert_eq!(request.await.unwrap().unwrap().bytes(), SVG.as_bytes());
    }
    assert_eq!(cache.stats().preview_generations, 3);
    assert_eq!(
        std::fs::read(directory.path().join("renderer.calls")).unwrap(),
        b"xxx"
    );
    cache.shutdown().await;
}
#[tokio::test]
#[ignore = "requires the built Rust renderer and local real-gift corpus"]
async fn real_corpus_generates_valid_cached_previews() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = tempfile::tempdir().unwrap();
    let (cache, _downloads) = MediaCache::with_renderer(
        directory.path(),
        Default::default(),
        root.join("target/debug/market-media-render"),
    )
    .await
    .unwrap();
    let corpus: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/scripts/fixtures/preview-corpus.json"
    )))
    .unwrap();
    for gift in corpus["gifts"].as_array().unwrap() {
        let id = gift["id"].as_str().unwrap();
        let document = DocumentId::new(id.parse().unwrap()).unwrap();
        let bytes = tokio::fs::read(root.join(format!("data/live-hub/media/{id}.tgs")))
            .await
            .unwrap();
        cache
            .store(
                MediaKey::animation(document),
                MediaAsset::animation(bytes).unwrap(),
            )
            .await
            .unwrap();
        let preview = cache.preview(document).await.unwrap();
        assert_eq!(preview.content_type(), "image/svg+xml");
        assert!(preview.etag().starts_with("W/"));
        assert_eq!(
            cache.preview(document).await.unwrap().etag(),
            preview.etag()
        );
    }
    assert_eq!(cache.stats().preview_generations, 43);
    cache.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn preview_survives_source_eviction_and_restart_without_a_provider() {
    let directory = tempfile::tempdir().unwrap();
    let renderer = helper(directory.path(), &format!("printf '%s' '{SVG}' > \"$2\""));
    let path = directory.path().join("cache");
    let limits = crate::MediaLimits {
        disk_bytes: SVG.len() as u64,
        memory_ttl: Duration::from_millis(30),
        ..Default::default()
    };
    let (cache, _downloads) = MediaCache::with_renderer(&path, limits.clone(), renderer)
        .await
        .unwrap();
    let document = DocumentId::new(42).unwrap();
    cache
        .store(MediaKey::animation(document), animation("evicted"))
        .await
        .unwrap();
    cache.preview(document).await.unwrap();
    assert!(
        !path.join("42.tgs").exists(),
        "writing the derivative evicts its older original"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    let before = cache.stats();
    cache.preview(document).await.unwrap();
    assert_eq!(cache.stats().memory_hits - before.memory_hits, 1);
    assert_eq!(cache.stats().disk_reads, before.disk_reads);
    cache.shutdown().await;
    let (cache, downloads) =
        MediaCache::with_renderer(&path, limits, directory.path().join("missing"))
            .await
            .unwrap();
    drop(downloads);
    assert_eq!(
        cache.preview(document).await.unwrap().bytes(),
        SVG.as_bytes()
    );
    assert_eq!(cache.stats().preview_generations, 0);
    assert_eq!(cache.stats().downloads, 0);
    cache.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn unsupported_render_is_negatively_cached_but_transient_failure_can_retry() {
    for (exit, expected, attempts) in [
        (2, MediaError::InvalidMedia, 1),
        (1, MediaError::Unavailable, 2),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let renderer = helper(
            directory.path(),
            &format!("printf 'conversion diagnostic' >&2\nexit {exit}"),
        );
        let (cache, _downloads) =
            MediaCache::with_renderer(directory.path().join("cache"), Default::default(), renderer)
                .await
                .unwrap();
        let document = DocumentId::new(7).unwrap();
        cache
            .store(MediaKey::animation(document), animation("invalid"))
            .await
            .unwrap();
        assert_eq!(cache.preview(document).await.unwrap_err(), expected);
        tokio::time::sleep(Duration::from_millis(2100)).await;
        assert_eq!(cache.preview(document).await.unwrap_err(), expected);
        assert_eq!(cache.stats().preview_generations, attempts);
        cache.shutdown().await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn replacing_a_source_rejects_late_derivatives_and_old_resident_updates() {
    let directory = tempfile::tempdir().unwrap();
    let (cache, _downloads) = MediaCache::open(directory.path()).await.unwrap();
    let document = DocumentId::new(7).unwrap();
    let old = animation("old");
    let new = animation("new");
    cache
        .store(MediaKey::animation(document), old.clone())
        .await
        .unwrap();
    cache
        .store(MediaKey::animation(document), new.clone())
        .await
        .unwrap();
    cache.remember(MediaKey::animation(document), &old);
    assert_eq!(
        cache
            .get(MediaKey::animation(document))
            .await
            .unwrap()
            .etag(),
        new.etag()
    );
    assert_eq!(
        cache
            .store(
                MediaKey::preview(document, old.digest),
                MediaAsset::preview(SVG.as_bytes().to_vec()).unwrap()
            )
            .await
            .unwrap_err(),
        MediaError::Unavailable
    );
    assert_eq!(
        cache.preview_key(document),
        Some(MediaKey::preview(document, new.digest))
    );
    cache.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn restart_checks_original_identity_before_reusing_an_old_derivative() {
    let directory = tempfile::tempdir().unwrap();
    let renderer = helper(directory.path(), &format!("printf '%s' '{SVG}' > \"$2\""));
    let path = directory.path().join("cache");
    let document = DocumentId::new(42).unwrap();
    let (cache, _downloads) =
        MediaCache::with_renderer(&path, Default::default(), renderer.clone())
            .await
            .unwrap();
    let old = animation("old");
    let old_key = MediaKey::preview(document, old.digest);
    cache
        .store(MediaKey::animation(document), old)
        .await
        .unwrap();
    cache.preview(document).await.unwrap();
    cache.shutdown().await;
    let new = animation("new");
    std::fs::write(path.join("42.tgs"), new.bytes()).unwrap();
    let (cache, _downloads) = MediaCache::with_renderer(&path, Default::default(), renderer)
        .await
        .unwrap();
    assert_eq!(
        cache.preview_key(document),
        Some(MediaKey::preview(document, new.digest))
    );
    assert!(cache.cached(old_key).await.unwrap().is_none());
    cache.preview(document).await.unwrap();
    assert_eq!(cache.stats().preview_generations, 1);
    cache.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn diagnostic_output_is_drained_without_blocking_the_renderer() {
    let directory = tempfile::tempdir().unwrap();
    let renderer = helper(
        directory.path(),
        "dd if=/dev/zero bs=1024 count=256 >&2 2>/dev/null\nexit 2",
    );
    let (cache, _downloads) =
        MediaCache::with_renderer(directory.path().join("cache"), Default::default(), renderer)
            .await
            .unwrap();
    let document = DocumentId::new(7).unwrap();
    cache
        .store(MediaKey::animation(document), animation("diagnostic"))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), cache.preview(document))
            .await
            .unwrap()
            .unwrap_err(),
        MediaError::InvalidMedia
    );
    cache.shutdown().await;
}
