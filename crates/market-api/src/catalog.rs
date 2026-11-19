//! HTTP presentation of catalog data. Domain media references never contain routes.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use market_gifts as gifts;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use utoipa::ToSchema;
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GiftMedia {
    pub thumbnail_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    pub animation_url: Option<String>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GiftCollection {
    pub id: gifts::CollectionId,
    pub name: String,
    pub slug: Option<gifts::CollectionSlug>,
    pub preview: Option<GiftMedia>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GiftModel {
    pub id: gifts::ModelId,
    pub name: String,
    pub media: Option<GiftMedia>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GiftPattern {
    pub id: gifts::PatternId,
    pub name: String,
    pub media: Option<GiftMedia>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GiftCatalog {
    pub collections: Arc<Vec<GiftCollection>>,
    pub venues: Arc<Vec<gifts::Venue>>,
    pub currencies: Arc<Vec<gifts::Currency>>,
    pub books: Arc<Vec<gifts::GiftBook>>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GiftCollectionCatalog {
    pub collection: GiftCollection,
    pub models: Arc<Vec<GiftModel>>,
    pub backdrops: Arc<Vec<gifts::GiftBackdrop>>,
    pub patterns: Arc<Vec<GiftPattern>>,
}
impl From<gifts::GiftMedia> for GiftMedia {
    fn from(value: gifts::GiftMedia) -> Self {
        let (kind, id, animated) = match value.target {
            gifts::MediaTarget::Model(id) => ("models", id.to_string(), true),
            gifts::MediaTarget::Pattern(id) => ("patterns", id.to_string(), false),
        };
        const COMPONENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
            .remove(b'.')
            .remove(b'-')
            .remove(b'_')
            .remove(b'~');
        let id = percent_encoding::utf8_percent_encode(&id, COMPONENT);
        let thumbnail_url = format!("/media/{kind}/{id}");
        let animation_url = animated.then(|| format!("{thumbnail_url}/animation"));
        let preview_url = animated.then(|| {
            format!(
                "{thumbnail_url}/preview?v={}",
                market_media::PREVIEW_VERSION
            )
        });
        Self {
            thumbnail_url,
            animation_url,
            preview_url,
        }
    }
}
impl From<gifts::GiftCollection> for GiftCollection {
    fn from(value: gifts::GiftCollection) -> Self {
        Self {
            id: value.id,
            slug: value.slug,
            name: value.name,
            preview: value.preview.map(Into::into),
        }
    }
}
impl From<gifts::GiftCatalog> for GiftCatalog {
    fn from(value: gifts::GiftCatalog) -> Self {
        Self {
            collections: value
                .collections
                .iter()
                .cloned()
                .map(Into::into)
                .collect::<Vec<_>>()
                .into(),
            venues: value.venues,
            currencies: value.currencies,
            books: value.books,
        }
    }
}
impl From<gifts::GiftCollectionCatalog> for GiftCollectionCatalog {
    fn from(value: gifts::GiftCollectionCatalog) -> Self {
        Self {
            collection: value.collection.into(),
            models: value
                .models
                .iter()
                .map(|m| GiftModel {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    media: m.media.clone().map(Into::into),
                })
                .collect::<Vec<_>>()
                .into(),
            patterns: value
                .patterns
                .iter()
                .map(|p| GiftPattern {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    media: p.media.clone().map(Into::into),
                })
                .collect::<Vec<_>>()
                .into(),
            backdrops: value.backdrops,
        }
    }
}
/// Internal representation namespace; update if URL/JSON mapping changes.
const REPRESENTATION: &[u8] = b"gift-catalog:rust-previews\0";
/// A stable 128-bit cache validator; tokens are opaque and provide no authentication.
pub(crate) fn revision(domain: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(REPRESENTATION);
    hash.update(domain);
    let digest = hash.finalize();
    URL_SAFE_NO_PAD.encode(&digest[..16])
}
pub(crate) fn versions(mut value: gifts::CatalogVersions) -> gifts::CatalogVersions {
    value.directory = revision(&value.directory);
    for version in value.collections.values_mut().flatten() {
        *version = revision(version);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revisions_are_short_stable_opaque_tokens_derived_from_the_full_content_version() {
        assert_eq!(revision("content"), "VY_j-TppoSZgChtVxAWjjw");
        let original = "0".repeat(64);
        let mut changed = original.clone();
        changed.replace_range(63.., "1");
        assert_ne!(revision(&original), revision(&changed));
        for domain in ["content", original.as_str(), changed.as_str()] {
            let token = revision(domain);
            assert_eq!(token.len(), 22);
            assert_eq!(URL_SAFE_NO_PAD.decode(&token).unwrap().len(), 16);
            assert!(
                token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            );
        }
        let versions = versions(gifts::CatalogVersions {
            directory: "content".into(),
            collections: [
                ("gift".into(), Some(original.clone())),
                ("missing".into(), None),
            ]
            .into(),
        });
        assert_eq!(versions.directory, revision("content"));
        assert_eq!(
            versions.collections[&gifts::CollectionId::from("gift")],
            Some(revision(&original))
        );
        assert_eq!(
            versions.collections[&gifts::CollectionId::from("missing")],
            None
        );
    }

    #[test]
    fn http_media_urls_are_encoded_by_the_presentation_layer() {
        let model = GiftMedia::from(gifts::GiftMedia {
            target: gifts::MediaTarget::Model(gifts::ModelId::new("a/b?#💎").unwrap()),
        });
        assert_eq!(model.thumbnail_url, "/media/models/a%2Fb%3F%23%F0%9F%92%8E");
        assert_eq!(
            model.preview_url,
            Some(format!(
                "{}/preview?v={}",
                model.thumbnail_url,
                market_media::PREVIEW_VERSION
            ))
        );
        assert_eq!(
            model.animation_url,
            Some(format!("{}/animation", model.thumbnail_url))
        );
        let pattern = GiftMedia::from(gifts::GiftMedia {
            target: gifts::MediaTarget::Pattern("pattern".into()),
        });
        assert_eq!(pattern.thumbnail_url, "/media/patterns/pattern");
        assert!(pattern.animation_url.is_none());
        assert!(pattern.preview_url.is_none());
        let json = serde_json::to_value(pattern).unwrap();
        assert!(json.get("target").is_none());
        assert!(json.get("preview_url").is_none());
        assert!(json["thumbnail_url"].is_string());
    }
}
