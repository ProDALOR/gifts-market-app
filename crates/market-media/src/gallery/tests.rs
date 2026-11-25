use super::*;
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect x="60" y="60" width="392" height="392" rx="70" fill="#e45a72"/></svg>"##;
fn model() -> MediaAsset {
    MediaAsset::parse(MediaKind::Preview, SVG.as_bytes().to_vec()).unwrap()
}
fn spec(count: usize, columns: u8) -> GiftGallery {
    GiftGallery {
        columns,
        cells: (0..count)
            .map(|i| GalleryCell {
                cover: Some(GiftCover {
                    model: DocumentId::new(1).unwrap(),
                    pattern: None,
                    palette: Some(CoverPalette {
                        center: 0x8981ce,
                        edge: 0x4b438d,
                        pattern: 0x3b3866,
                    }),
                }),
                title: "Pink Latex & Friends".into(),
                identity: format!("#{}", 1336 + i),
                price: format!("{} GRAM", 7457 + i),
            })
            .collect(),
    }
}
#[test]
fn galleries_render_six_trades_and_twelve_gifts_with_vector_labels_and_validated_sizes() {
    for (count, columns) in [(1, 3), (6, 2), (12, 3)] {
        let gallery = spec(count, columns);
        let svg = compose(&gallery, &[(DocumentId::new(1).unwrap(), model())].into()).unwrap();
        assert!(!svg.contains("<text"));
        assert_eq!(svg.matches("clipPath id=\"cell-").count(), count);
        assert!(svg.contains("c0-cover-bg"));
        let png = render_gallery_png(svg.as_bytes()).unwrap();
        validate_png(&png).unwrap();
        let pixels = resvg::tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert_eq!(
            (pixels.width(), pixels.height()),
            (1200, height(count, columns))
        );
        assert!(pixels.pixels().iter().all(|p| p.alpha() == 255));
        assert!(
            pixels
                .pixels()
                .iter()
                .any(|p| p.red() > 240 && p.green() > 240 && p.blue() > 240),
            "font glyphs must be visible"
        );
        assert!(
            render_gallery_png(
                svg.replace("<rect", "<image href=\"file:///etc/passwd\"")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(validate_png(&png[..png.len() / 2]).is_err());
    }
    for bad in [spec(0, 3), spec(13, 3), spec(6, 0), spec(12, 2)] {
        assert!(bad.validate().is_err());
    }
    let mut bad = spec(1, 3);
    bad.cells[0].title = "a".repeat(257);
    assert!(bad.validate().is_err());
}
#[test]
fn gallery_cache_keys_cover_order_prices_layout_and_source_identity() {
    let id = DocumentId::new(1).unwrap();
    let sources = [(id, [1; 32])].into();
    let base = spec(2, 3);
    let original = key(id, &base, &sources);
    let mut changed = base.clone();
    changed.cells[0].price = "1 000 GRAM".into();
    assert_ne!(original, key(id, &changed, &sources));
    changed = base.clone();
    changed.cells.swap(0, 1);
    assert_ne!(original, key(id, &changed, &sources));
    changed = base.clone();
    changed.columns = 2;
    assert_ne!(original, key(id, &changed, &sources));
    changed = base.clone();
    changed.cells[0]
        .cover
        .as_mut()
        .unwrap()
        .palette
        .as_mut()
        .unwrap()
        .center ^= 1;
    assert_ne!(original, key(id, &changed, &sources));
    assert_ne!(original, key(id, &base, &[(id, [2; 32])].into()));
}
async fn seed(cache: &MediaCache, name: &str) {
    use std::io::Write;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    write!(gzip,"{{\"v\":\"5.7.4\",\"w\":512,\"h\":512,\"fr\":60,\"ip\":0,\"op\":180,\"nm\":\"{name}\",\"layers\":[]}}").unwrap();
    let source = MediaAsset::animation(gzip.finish().unwrap()).unwrap();
    let id = DocumentId::new(1).unwrap();
    let key = MediaKey::preview(id, source.digest);
    cache.store(MediaKey::animation(id), source).await.unwrap();
    cache.store(key, model()).await.unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn gallery_jobs_coalesce_invalidate_and_reuse_disk_without_svg_reload() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let gallery = spec(1, 3);
    let png = render_gallery_png(
        compose(&gallery, &[(DocumentId::new(1).unwrap(), model())].into())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    std::fs::write(dir.path().join("fixture.png"), png).unwrap();
    let renderer = dir.path().join("renderer");
    std::fs::write(
        &renderer,
        "#!/bin/sh\nsleep 0.05\ncp \"$(dirname \"$0\")/fixture.png\" \"$2\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&renderer, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = dir.path().join("cache");
    let (cache, _) = MediaCache::with_renderer(&path, Default::default(), renderer.clone())
        .await
        .unwrap();
    seed(&cache, "first").await;
    let values =
        futures_util::future::join_all((0..12).map(|_| cache.gallery(gallery.clone()))).await;
    let first = values[0].as_ref().unwrap().etag().to_owned();
    assert!(values.into_iter().all(|v| v.unwrap().etag() == first));
    assert_eq!(cache.stats().gallery_generations, 1);
    let mut changed = gallery.clone();
    changed.cells[0].price = "12 GRAM".into();
    cache.gallery(changed).await.unwrap();
    assert_eq!(cache.stats().gallery_generations, 2);
    seed(&cache, "changed").await;
    cache.gallery(gallery.clone()).await.unwrap();
    assert_eq!(cache.stats().gallery_generations, 3);
    cache.shutdown().await;
    assert_eq!(
        cache.gallery(gallery.clone()).await.unwrap_err(),
        MediaError::Unavailable
    );
    for entry in std::fs::read_dir(&path).unwrap() {
        let file = entry.unwrap().path();
        if file.extension().is_some_and(|ext| ext == "svg") {
            std::fs::remove_file(file).unwrap();
        }
    }
    let (reopened, _) = MediaCache::with_renderer(&path, Default::default(), renderer)
        .await
        .unwrap();
    reopened.gallery(gallery).await.unwrap();
    assert_eq!(reopened.stats().gallery_generations, 0);
    assert_eq!(reopened.stats().downloads, 0);
    let mut blank = spec(1, 3);
    blank.cells[0].cover = None;
    reopened.gallery(blank).await.unwrap();
    assert_eq!(
        reopened.stats().downloads,
        0,
        "broad bids need no media download"
    );
    reopened.shutdown().await;
}
