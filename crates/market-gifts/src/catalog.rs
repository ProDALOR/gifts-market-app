//! Gift collections, appearance choices and catalog revision contracts.
use crate::{
    BackdropId, CollectionId, CurrencyId, GiftBook, GiftMedia, ModelId, PatternId, RgbColor,
    VenueId,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftCollection {
    pub id: CollectionId,
    pub name: String,
    pub preview: Option<GiftMedia>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftModel {
    pub id: ModelId,
    pub name: String,
    pub media: Option<GiftMedia>,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftPattern {
    pub id: PatternId,
    pub name: String,
    pub media: Option<GiftMedia>,
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

/// Content revisions for catalog cache revalidation; not replica checkpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CatalogVersions {
    pub directory: String,
    /// Null means that collection no longer exists as a gift collection.
    pub collections: std::collections::BTreeMap<CollectionId, Option<String>>,
}
