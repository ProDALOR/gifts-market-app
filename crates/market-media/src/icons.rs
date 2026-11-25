//! Curated Telegram UI documents shared by native bot emoji and static web icons.
use crate::DocumentId;
use std::sync::OnceLock;

macro_rules! icons {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub enum UiIcon { $($variant),+ }
        impl UiIcon {
            pub const ALL: &[Self] = &[$(Self::$variant),+];
            pub fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
        }
    };
}
icons! {
    Collection => "collection", Model => "model", Backdrop => "backdrop",
    ModelBackdrop => "model_backdrop", Symbol => "symbol", Estimate => "estimate",
    History => "history", CurrentGift => "current_gift", MarketLink => "market_link",
    Gram => "gram", Stars => "stars", Warning => "warning", Search => "search",
    Close => "close", Info => "info", Up => "up", Filter => "filter", Sort => "sort",
    User => "user", Refresh => "refresh",
    Market => "market", Portfolio => "portfolio",
}

#[derive(Debug, Clone, Copy)]
pub struct IconDefinition {
    pub document: DocumentId,
    /// Telegram requires this exact alternative for custom emoji entities.
    pub alternative: &'static str,
}
impl UiIcon {
    /// Public routes accept only curated names, never arbitrary document IDs.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|icon| icon.name() == name)
    }
    pub fn definition(self) -> IconDefinition {
        static MANIFEST: OnceLock<serde_json::Value> = OnceLock::new();
        let manifest = MANIFEST.get_or_init(|| {
            serde_json::from_str(include_str!("../assets/icons.json"))
                .expect("embedded icon manifest is valid JSON")
        });
        let entry = &manifest["icons"][self.name()];
        IconDefinition {
            document: DocumentId::new(
                entry["document_id"]
                    .as_str()
                    .expect("icon document ID")
                    .parse()
                    .expect("numeric icon document ID"),
            )
            .expect("positive icon document ID"),
            alternative: entry["alternative"]
                .as_str()
                .expect("verified icon alternative"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn icon_previews_reuse_svg_cache_and_survive_offline_restart() {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let document = UiIcon::Gram.definition().document;
        let mut tgs = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        tgs.write_all(br#"{"v":"5.7.4","w":512,"h":512,"fr":60,"ip":0,"op":60,"layers":[]}"#)
            .unwrap();
        let source = crate::MediaAsset::animation(tgs.finish().unwrap()).unwrap();
        let image = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M0 0L1 1" fill="#fff"/></svg>"##;
        std::fs::write(
            directory.path().join(format!("{}.tgs", document.get())),
            source.bytes(),
        )
        .unwrap();
        let key = crate::MediaKey::preview(document, source.digest);
        std::fs::write(
            directory
                .path()
                .join(format!("{}.{}", document.get(), key.extension())),
            image,
        )
        .unwrap();
        for _ in 0..2 {
            let (cache, downloads) = crate::MediaCache::open(directory.path()).await.unwrap();
            drop(downloads);
            for _ in 0..3 {
                let icon = cache.preview(document).await.unwrap();
                assert_eq!(icon.bytes(), image);
                assert_eq!(icon.content_type(), "image/svg+xml");
            }
            assert_eq!(cache.stats().downloads, 0);
            assert_eq!(cache.stats().disk_reads, 1);
            cache.shutdown().await;
        }
    }
    #[test]
    fn shared_manifest_is_complete_and_only_named_icons_are_public() {
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../assets/icons.json")).unwrap();
        assert_eq!(raw["icons"].as_object().unwrap().len(), UiIcon::ALL.len());
        assert_eq!(
            raw["preview_version"].as_u64().unwrap().to_string(),
            crate::PREVIEW_VERSION
        );
        let mut documents = std::collections::BTreeSet::new();
        for &icon in UiIcon::ALL {
            assert_eq!(UiIcon::from_name(icon.name()), Some(icon));
            let definition = icon.definition();
            assert!(documents.insert(definition.document.get()));
            assert!(!definition.alternative.trim().is_empty());
            assert!(definition.alternative.chars().count() <= 32);
            assert!(!definition.alternative.chars().any(char::is_control));
        }
        for unknown in [
            "",
            "tgiosicons",
            "5773677501825945508",
            "../collection",
            "Collection",
        ] {
            assert!(UiIcon::from_name(unknown).is_none());
        }
    }
}
