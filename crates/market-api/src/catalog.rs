//! HTTP presentation of catalog data. Domain media references never contain routes.
use market_gifts as gifts;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GiftMedia {
    pub thumbnail_url: String,
    pub animation_url: Option<String>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GiftCollection {
    pub id: gifts::CollectionId,
    pub name: String,
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
        Self {
            thumbnail_url,
            animation_url,
        }
    }
}
impl From<gifts::GiftCollection> for GiftCollection {
    fn from(value: gifts::GiftCollection) -> Self {
        Self {
            id: value.id,
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
/// Namespace content revisions by the public representation. Bump if URL/JSON mapping changes.
pub(crate) fn revision(domain: String) -> String {
    format!("catalog-v1-{domain}")
}
pub(crate) fn versions(mut value: gifts::CatalogVersions) -> gifts::CatalogVersions {
    value.directory = revision(value.directory);
    for version in value.collections.values_mut().flatten() {
        *version = revision(std::mem::take(version));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn http_media_urls_are_encoded_by_the_presentation_layer() {
        let model = GiftMedia::from(gifts::GiftMedia {
            target: gifts::MediaTarget::Model(gifts::ModelId::new("a/b?#💎").unwrap()),
        });
        assert_eq!(model.thumbnail_url, "/media/models/a%2Fb%3F%23%F0%9F%92%8E");
        assert_eq!(
            model.animation_url,
            Some(format!("{}/animation", model.thumbnail_url))
        );
        let pattern = GiftMedia::from(gifts::GiftMedia {
            target: gifts::MediaTarget::Pattern("pattern".into()),
        });
        assert_eq!(pattern.thumbnail_url, "/media/patterns/pattern");
        assert!(pattern.animation_url.is_none());
        let json = serde_json::to_value(pattern).unwrap();
        assert!(json.get("target").is_none());
        assert!(json["thumbnail_url"].is_string());
    }
}
