//! Gift-market language shared by actors and transports. No hub, storage, HTTP or Telegram client dependencies.
mod ids;
mod money;
pub mod valuation;
pub use ids::*;
use jiff::Timestamp;
pub use money::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftBook {
    pub venue_id: VenueId,
    pub collection_id: CollectionId,
    pub currency_id: CurrencyId,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftOrderKind {
    #[default]
    Ask,
    Bid,
    Executed,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftSort {
    Identity,
    #[default]
    PriceAsc,
    PriceDesc,
    TimeAsc,
    TimeDesc,
    NumberAsc,
    NumberDesc,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PriceRange {
    pub min: Option<Amount>,
    pub max: Option<Amount>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftNumberRange {
    #[cfg_attr(feature = "schema", schema(minimum = 1))]
    pub min: Option<u32>,
    #[cfg_attr(feature = "schema", schema(minimum = 1))]
    pub max: Option<u32>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftCriteria {
    pub models: Vec<ModelId>,
    pub backdrops: Vec<BackdropId>,
    pub patterns: Vec<PatternId>,
    pub number: Option<GiftNumberRange>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftFilters {
    pub collection_id: CollectionId,
    pub currency_id: CurrencyId,
    pub venue_id: Option<VenueId>,
    #[serde(default)]
    pub models: Vec<ModelId>,
    #[serde(default)]
    pub backdrops: Vec<BackdropId>,
    #[serde(default)]
    pub patterns: Vec<PatternId>,
    pub number: Option<GiftNumberRange>,
    pub price: Option<PriceRange>,
}
impl GiftFilters {
    pub fn new(collection_id: CollectionId, currency_id: CurrencyId) -> Self {
        Self {
            collection_id,
            currency_id,
            venue_id: None,
            models: vec![],
            backdrops: vec![],
            patterns: vec![],
            number: None,
            price: None,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftSide {
    #[default]
    Ask,
    Bid,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ReadConsistency {
    #[default]
    Snapshot,
    Live,
}
fn default_limit() -> u32 {
    50
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftOrdersQuery {
    pub filters: GiftFilters,
    #[serde(default)]
    pub side: GiftSide,
    #[serde(default)]
    pub sort: GiftSort,
    #[serde(default)]
    pub consistency: ReadConsistency,
    #[serde(default = "default_limit")]
    #[cfg_attr(feature = "schema", schema(minimum = 1, maximum = 200))]
    pub limit: u32,
    #[serde(default)]
    pub page_token: String,
}
impl GiftOrdersQuery {
    pub fn new(filters: GiftFilters) -> Self {
        Self {
            filters,
            side: Default::default(),
            sort: Default::default(),
            consistency: Default::default(),
            limit: default_limit(),
            page_token: String::new(),
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Period {
    #[cfg_attr(feature = "schema", schema(value_type = Option<String>, format = DateTime))]
    pub from: Option<Timestamp>,
    #[cfg_attr(feature = "schema", schema(value_type = Option<String>, format = DateTime))]
    pub until: Option<Timestamp>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftHistoryQuery {
    pub filters: GiftFilters,
    #[serde(default = "history_sort")]
    pub sort: GiftSort,
    pub period: Option<Period>,
    #[serde(default = "default_limit")]
    #[cfg_attr(feature = "schema", schema(minimum = 1, maximum = 200))]
    pub limit: u32,
    #[serde(default)]
    pub page_token: String,
}
fn history_sort() -> GiftSort {
    GiftSort::Identity
}
impl GiftHistoryQuery {
    pub fn new(filters: GiftFilters) -> Self {
        Self {
            filters,
            sort: GiftSort::Identity,
            period: None,
            limit: default_limit(),
            page_token: String::new(),
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftTraits {
    pub model: Option<ModelId>,
    pub backdrop: Option<BackdropId>,
    pub pattern: Option<PatternId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CollectibleGift {
    pub id: GiftId,
    pub collection_id: CollectionId,
    pub number: Option<u32>,
    pub traits: GiftTraits,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftTarget {
    Gift {
        gift: CollectibleGift,
    },
    Criteria {
        criteria: GiftCriteria,
    },
    /// Unsupported hub constraints must never be displayed as unrestricted bids.
    Unsupported {
        reason: String,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftOrder {
    pub id: OrderId,
    pub book: GiftBook,
    pub mode: GiftOrderKind,
    pub target: GiftTarget,
    pub price: Amount,
    pub quantity: u32,
    #[cfg_attr(feature = "schema", schema(value_type = String, format = DateTime))]
    pub observed_at: Timestamp,
    #[cfg_attr(feature = "schema", schema(value_type = Option<String>, format = DateTime))]
    pub expires_at: Option<Timestamp>,
    #[cfg_attr(feature = "schema", schema(value_type = Option<String>, format = DateTime))]
    pub executed_at: Option<Timestamp>,
    pub parent_order_id: Option<OrderId>,
    pub market_url: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Page<T> {
    pub data: Vec<T>,
    pub next_page_token: String,
    pub cursor: String,
    pub sync: SyncStatus,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum SyncState {
    Starting,
    Syncing,
    Live,
    Stale,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct SyncStatus {
    pub ready: bool,
    pub state: SyncState,
    #[cfg_attr(feature = "schema", schema(value_type = Option<String>, format = DateTime))]
    pub updated_at: Option<Timestamp>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftCollection {
    pub id: CollectionId,
    pub name: String,
    pub preview: Option<GiftMedia>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftMedia {
    pub thumbnail_url: String,
    pub animation_url: Option<String>,
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
    pub cursor: String,
    pub sync: SyncStatus,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftCollectionCatalog {
    pub collection: GiftCollection,
    pub models: std::sync::Arc<Vec<GiftModel>>,
    pub backdrops: std::sync::Arc<Vec<GiftBackdrop>>,
    pub patterns: std::sync::Arc<Vec<GiftPattern>>,
    pub cursor: String,
    pub sync: SyncStatus,
}
#[derive(Debug, Clone)]
pub enum MediaTarget {
    Model(ModelId),
    Pattern(PatternId),
}

#[cfg(test)]
mod tests;
