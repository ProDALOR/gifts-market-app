//! Gift collections, appearance choices and catalog revision contracts.
use crate::{
    BackdropId, CollectionId, CollectionSlug, CurrencyId, GiftBook, GiftMedia, ModelId, PatternId,
    RgbColor, VenueId,
};
use serde::{Deserialize, Serialize};

/// Exact attribute rarity in parts per thousand: 5 means 0.5%, 1000 means 100%.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct RarityPermille(u16);
#[cfg(feature = "schema")]
impl utoipa::PartialSchema for RarityPermille {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Integer)
            .minimum(Some(0.0))
            .maximum(Some(1000.0))
            .into()
    }
}
#[cfg(feature = "schema")]
impl utoipa::ToSchema for RarityPermille {}
impl TryFrom<u16> for RarityPermille {
    type Error = crate::InvalidValue;
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value <= 1000 {
            Ok(Self(value))
        } else {
            Err(crate::InvalidValue(
                "rarity must be between 0 and 1000 permille",
            ))
        }
    }
}
impl From<RarityPermille> for u16 {
    fn from(value: RarityPermille) -> Self {
        value.0
    }
}

/// Numeric rarity and crafted rarity tiers have distinct meanings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftRarity {
    Percentage { permille: RarityPermille },
    Uncommon,
    Rare,
    Epic,
    Legendary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftCollection {
    pub id: CollectionId,
    pub name: String,
    pub slug: Option<CollectionSlug>,
    pub preview: Option<GiftMedia>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftModel {
    pub id: ModelId,
    pub name: String,
    pub media: Option<GiftMedia>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rarity: Option<GiftRarity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct BackdropPalette {
    pub center: RgbColor,
    pub edge: RgbColor,
    pub pattern: RgbColor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftBackdrop {
    pub id: BackdropId,
    pub name: String,
    pub palette: Option<BackdropPalette>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rarity: Option<GiftRarity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftPattern {
    pub id: PatternId,
    pub name: String,
    pub media: Option<GiftMedia>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rarity: Option<GiftRarity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Venue {
    pub id: VenueId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Currency {
    pub id: CurrencyId,
    pub name: String,
    pub decimals: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftCatalog {
    pub collections: std::sync::Arc<Vec<GiftCollection>>,
    pub venues: std::sync::Arc<Vec<Venue>>,
    pub currencies: std::sync::Arc<Vec<Currency>>,
    pub books: std::sync::Arc<Vec<GiftBook>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftCollectionCatalog {
    pub collection: GiftCollection,
    pub models: std::sync::Arc<Vec<GiftModel>>,
    pub backdrops: std::sync::Arc<Vec<GiftBackdrop>>,
    pub patterns: std::sync::Arc<Vec<GiftPattern>>,
}

/// Content revisions for catalog cache revalidation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CatalogVersions {
    pub directory: String,
    /// Null means that collection no longer exists as a gift collection.
    pub collections: std::collections::BTreeMap<CollectionId, Option<String>>,
}
