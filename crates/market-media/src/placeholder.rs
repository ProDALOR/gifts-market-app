//! Small passive masks, independent of Telegram encoding and the animation renderer.
use crate::{MAX_PLACEHOLDER_BYTES, MediaError};
fn path(value: &str) -> Result<kurbo::BezPath, MediaError> {
    if value.is_empty() || value.len() > MAX_PLACEHOLDER_BYTES / 2 {
        return Err(MediaError::InvalidMedia);
    }
    let path = kurbo::BezPath::from_svg(value).map_err(|_| MediaError::InvalidMedia)?;
    if path.is_empty()
        || path.elements().iter().any(|element| {
            use kurbo::PathEl::*;
            let finite = |p: kurbo::Point| p.x.is_finite() && p.y.is_finite();
            match element {
                MoveTo(p) | LineTo(p) => !finite(*p),
                QuadTo(a, b) => !finite(*a) || !finite(*b),
                CurveTo(a, b, c) => !finite(*a) || !finite(*b) || !finite(*c),
                ClosePath => false,
            }
        })
    {
        return Err(MediaError::InvalidMedia);
    }
    Ok(path)
}
pub(crate) fn svg(value: &str) -> Result<Vec<u8>, MediaError> {
    let path = path(value)?.to_svg();
    let bytes = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path fill="#ffffff" d="{path}"/></svg>"##).into_bytes();
    if bytes.len() > MAX_PLACEHOLDER_BYTES {
        return Err(MediaError::InvalidMedia);
    }
    Ok(bytes)
}
pub(crate) fn validate(bytes: &[u8]) -> Result<(), MediaError> {
    if bytes.len() > MAX_PLACEHOLDER_BYTES {
        return Err(MediaError::InvalidMedia);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MediaError::InvalidMedia)?;
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            nodes_limit: 8,
            ..Default::default()
        },
    )
    .map_err(|_| MediaError::InvalidMedia)?;
    let mut elements = doc.descendants().filter(|node| node.is_element());
    let root = elements.next().ok_or(MediaError::InvalidMedia)?;
    let mask = elements.next().ok_or(MediaError::InvalidMedia)?;
    if root.tag_name().namespace() != Some("http://www.w3.org/2000/svg")
        || mask.tag_name().namespace() != Some("http://www.w3.org/2000/svg")
        || root.attribute("viewBox") != Some("0 0 512 512")
        || root.tag_name().name() != "svg"
        || mask.tag_name().name() != "path"
        || mask.attribute("fill") != Some("#ffffff")
        || elements.next().is_some()
        || root
            .attributes()
            .any(|a| a.namespace().is_some() || a.name() != "viewBox")
        || mask
            .attributes()
            .any(|a| a.namespace().is_some() || !matches!(a.name(), "fill" | "d"))
    {
        return Err(MediaError::InvalidMedia);
    }
    path(mask.attribute("d").ok_or(MediaError::InvalidMedia)?)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::MediaAsset;
    #[test]
    fn masks_are_passive_bounded_paths_without_a_background() {
        let mask = MediaAsset::placeholder("M0,0L63,0L63,63z").unwrap();
        assert_eq!(mask.content_type(), "image/svg+xml");
        assert!(mask.etag().starts_with("W/"));
        for source in ["", "M0\"/><script/>", "M1e309,0L1,1z"] {
            assert!(MediaAsset::placeholder(source).is_err());
        }
        let svg = std::str::from_utf8(mask.bytes()).unwrap();
        for invalid in [
            svg.replace("<path", "<rect"),
            svg.replace("#ffffff", "#000000"),
            svg.replace("<path", "<path opacity=\"0\""),
            svg.replace("</svg>", "<animate/></svg>"),
        ] {
            assert!(MediaAsset::placeholder_svg(invalid.into_bytes()).is_err());
        }
        assert!(MediaAsset::placeholder_svg(vec![b' '; MAX_PLACEHOLDER_BYTES + 1]).is_err());
    }
}

#[cfg(test)]
mod cache_tests {
    use crate::{DocumentId, MediaAsset, MediaCache, MediaError, MediaKey, MediaKind};
    #[tokio::test]
    async fn outlines_coalesce_negative_cache_and_survive_offline_restart() {
        let directory = tempfile::tempdir().unwrap();
        let (cache, mut downloads) = MediaCache::open(directory.path()).await.unwrap();
        let worker = tokio::spawn(async move {
            downloads
                .run(|key| async move {
                    assert_eq!(key.kind(), MediaKind::Placeholder);
                    if key.document().get() == 7 {
                        return Err(MediaError::NotFound);
                    }
                    MediaAsset::placeholder("M0,0L30,0L30,30z")
                })
                .await;
        });
        let key = MediaKey::placeholder(DocumentId::new(42).unwrap());
        for result in futures_util::future::join_all((0..20).map(|_| cache.get(key))).await {
            assert_eq!(result.unwrap().content_type(), "image/svg+xml");
        }
        let missing = MediaKey::placeholder(DocumentId::new(7).unwrap());
        for _ in 0..3 {
            assert_eq!(cache.get(missing).await.unwrap_err(), MediaError::NotFound);
        }
        assert_eq!(cache.stats().downloads, 2);
        let etag = cache.get(key).await.unwrap().etag().to_owned();
        cache.shutdown().await;
        worker.await.unwrap();
        let (cache, downloads) = MediaCache::open(directory.path()).await.unwrap();
        drop(downloads);
        assert_eq!(cache.get(key).await.unwrap().etag(), etag);
        assert_eq!(cache.stats().downloads, 0);
        assert_eq!(cache.stats().preview_generations, 0);
        cache.shutdown().await;
    }
}
