use super::*;
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><defs><linearGradient id="paint"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#ff0000"/></linearGradient></defs><rect width="512" height="512" fill="url(#paint)"/></svg>"##;
fn model() -> MediaAsset {
    MediaAsset::parse(crate::MediaKind::Preview, SVG.as_bytes().to_vec()).unwrap()
}
fn spec() -> GiftCover {
    GiftCover {
        model: DocumentId::new(1).unwrap(),
        pattern: Some(DocumentId::new(2).unwrap()),
        palette: Some(CoverPalette {
            center: 0x112233,
            edge: 0x334455,
            pattern: 0x00ff00,
        }),
    }
}
#[test]
fn covers_render_vector_models_tinted_symbols_and_opaque_backdrops() {
    let svg = compose(&model(), Some(&model()), spec().palette).unwrap();
    assert!(svg.contains("id=\"model-paint\""));
    assert!(svg.contains("id=\"pattern-paint\""));
    assert_eq!(svg.matches("mask=\"url(#cover-pattern)\"").count(), 18);
    let bytes = render_cover_png(svg.as_bytes()).unwrap();
    validate_png(&bytes).unwrap();
    let pixels = resvg::tiny_skia::Pixmap::decode_png(&bytes).unwrap();
    assert_eq!((pixels.width(), pixels.height()), (1200, 600));
    assert!(pixels.pixels().iter().all(|p| p.alpha() == 255));
    assert_eq!(pixels.pixel(600, 300).unwrap().red(), 255);
    assert!(
        svg.contains("translate(0 -35)"),
        "native wallpaper is center-cropped for the 2:1 cover"
    );
    let without_symbols =
        render_cover_png(compose(&model(), None, spec().palette).unwrap().as_bytes()).unwrap();
    let plain = resvg::tiny_skia::Pixmap::decode_png(&without_symbols).unwrap();
    let tile = pixels.pixel(866, 81).unwrap();
    assert!(
        tile.green() > plain.pixel(866, 81).unwrap().green(),
        "symbol tint survives the centered crop"
    );
    // A full-square source occupies 456 px (76% of 600), centered at (600, 300).
    for (x, y) in [(600, 73), (600, 526), (373, 300), (826, 300)] {
        assert_eq!(pixels.pixel(x, y).unwrap().red(), 255);
    }
    for (x, y) in [(600, 70), (600, 530), (370, 300), (830, 300)] {
        assert_ne!(pixels.pixel(x, y).unwrap().red(), 255);
    }
    let old_size = resvg::tiny_skia::Pixmap::new(1200, 800)
        .unwrap()
        .encode_png()
        .unwrap();
    assert!(
        validate_png(&old_size).is_err(),
        "old cover dimensions must not be reused"
    );
    assert!(validate_png(&bytes[..bytes.len() / 2]).is_err());
    let hostile = svg.replace("<rect", "<image href=\"file:///etc/passwd\"");
    assert!(render_cover_png(hostile.as_bytes()).is_err());
}
#[test]
fn cover_keys_include_every_visual_input() {
    let cover = spec();
    let base = cover_key(cover, [1; 32], Some([2; 32]));
    let mut changed = cover;
    changed.palette.as_mut().unwrap().center += 1;
    assert_ne!(base, cover_key(changed, [1; 32], Some([2; 32])));
    changed = cover;
    changed.pattern = Some(DocumentId::new(3).unwrap());
    assert_ne!(base, cover_key(changed, [1; 32], Some([2; 32])));
    assert_ne!(base, cover_key(cover, [9; 32], Some([2; 32])));
    assert_ne!(base, cover_key(cover, [1; 32], Some([9; 32])));
    assert_ne!(base, cover_key(cover, [1; 32], None));
    changed = cover;
    changed.palette = None;
    let ignored = cover_key(changed, [1; 32], Some([2; 32]));
    changed.pattern = None;
    assert_eq!(
        ignored,
        cover_key(changed, [1; 32], None),
        "no backdrop means no symbol work or distinct key"
    );
    changed = cover;
    changed.pattern = None;
    let ignored_color = cover_key(changed, [1; 32], None);
    changed.palette.as_mut().unwrap().pattern ^= 0xffffff;
    assert_eq!(
        ignored_color,
        cover_key(changed, [1; 32], None),
        "unused tint does not duplicate a cover"
    );
    let neutral = compose(&model(), None, None).unwrap();
    assert!(render_cover_png(neutral.as_bytes()).is_ok());
}
async fn seed(cache: &MediaCache, id: DocumentId, name: &str) {
    use std::io::Write;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    write!(gzip,"{{\"v\":\"5.7.4\",\"w\":512,\"h\":512,\"fr\":60,\"ip\":0,\"op\":180,\"nm\":\"{name}\",\"layers\":[]}}").unwrap();
    let source = MediaAsset::animation(gzip.finish().unwrap()).unwrap();
    let key = MediaKey::preview(id, source.digest);
    cache.store(MediaKey::animation(id), source).await.unwrap();
    cache.store(key, model()).await.unwrap();
}
#[cfg(unix)]
fn helper(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("renderer");
    std::fs::write(
        &path,
        format!("#!/bin/sh\nprintf x >> \"$0.calls\"\n{body}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}
#[cfg(unix)]
#[tokio::test]
async fn covers_coalesce_invalidate_and_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let png = render_cover_png(
        compose(&model(), Some(&model()), spec().palette)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    std::fs::write(dir.path().join("fixture.png"), &png).unwrap();
    let renderer = helper(
        dir.path(),
        "sleep 0.05\ncp \"$(dirname \"$0\")/fixture.png\" \"$2\"",
    );
    let path = dir.path().join("cache");
    let (cache, _) = MediaCache::with_renderer(&path, Default::default(), renderer)
        .await
        .unwrap();
    seed(&cache, spec().model, "first").await;
    seed(&cache, spec().pattern.unwrap(), "pattern").await;
    let results = futures_util::future::join_all((0..20).map(|_| cache.cover(spec()))).await;
    for result in results {
        assert_eq!(result.unwrap().bytes(), png);
    }
    assert_eq!(cache.stats().cover_generations, 1);
    let mut other = spec();
    other.palette.as_mut().unwrap().center += 1;
    cache.cover(other).await.unwrap();
    assert_eq!(cache.stats().cover_generations, 2);
    seed(&cache, spec().model, "changed source").await;
    cache.cover(spec()).await.unwrap();
    assert_eq!(
        cache.stats().cover_generations,
        3,
        "replaced TGS invalidates the cover even before SVG reads"
    );
    seed(&cache, spec().model, "changed visual source").await;
    let updated = MediaAsset::parse(
        crate::MediaKind::Preview,
        SVG.replace("#ff0000", "#0000ff").into_bytes(),
    )
    .unwrap();
    cache
        .store(cache.preview_key(spec().model).unwrap(), updated)
        .await
        .unwrap();
    cache.cover(spec()).await.unwrap();
    assert_eq!(
        cache.stats().cover_generations,
        4,
        "changed source content invalidates the cover"
    );
    cache.shutdown().await;
    for entry in std::fs::read_dir(&path).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "svg") {
            std::fs::remove_file(path).unwrap();
        }
    }
    let (reopened, _) =
        MediaCache::with_renderer(&path, Default::default(), dir.path().join("missing-helper"))
            .await
            .unwrap();
    assert_eq!(reopened.cover(spec()).await.unwrap().bytes(), png);
    assert_eq!(reopened.stats().cover_generations, 0);
    assert_eq!(
        reopened.stats().preview_generations,
        0,
        "evicted SVGs are not rebuilt for a cached PNG"
    );
    assert_eq!(
        reopened.stats().downloads,
        0,
        "cached source identities require no Telegram requests"
    );
    reopened.shutdown().await;
}
#[cfg(unix)]
#[tokio::test]
async fn cancelling_a_cover_kills_its_renderer_and_shutdown_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let renderer = helper(dir.path(), "sleep 10");
    let (cache, _) = MediaCache::with_renderer(
        dir.path().join("cache"),
        Default::default(),
        renderer.clone(),
    )
    .await
    .unwrap();
    seed(&cache, spec().model, "first").await;
    seed(&cache, spec().pattern.unwrap(), "pattern").await;
    let reader = cache.clone();
    let task = tokio::spawn(async move { reader.cover(spec()).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !renderer.with_extension("calls").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    tokio::time::timeout(std::time::Duration::from_secs(2), cache.shutdown())
        .await
        .unwrap();
}

#[test]
fn cover_disk_entries_obey_quotas_and_remove_only_obsolete_versions() {
    let dir = tempfile::tempdir().unwrap();
    let png =
        render_cover_png(compose(&model(), None, spec().palette).unwrap().as_bytes()).unwrap();
    let asset = MediaAsset::parse(crate::MediaKind::Cover, png.clone()).unwrap();
    let store = crate::disk::DiskStore::open(
        dir.path().to_owned(),
        crate::MediaLimits {
            disk_bytes: png.len() as u64 * 2 + 1,
            ..Default::default()
        },
    )
    .unwrap();
    for id in 1..=3 {
        store
            .write(
                MediaKey::cover(DocumentId::new(id).unwrap(), [id as u8; 32]),
                &asset,
            )
            .unwrap();
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    drop(store);
    let old = format!("99.cover-2-{}.png", "ab".repeat(32));
    std::fs::write(dir.path().join(&old), &png).unwrap();
    std::fs::write(dir.path().join("keep.png"), b"unrelated").unwrap();
    let _store = crate::disk::DiskStore::open(dir.path().to_owned(), Default::default()).unwrap();
    assert!(!dir.path().join(old).exists());
    assert!(dir.path().join("keep.png").exists());
}
