//! Gift identities, traits, order targets and exact market quotes.
use crate::{
    Amount, BackdropId, CollectionId, CurrencyId, GiftId, GiftNumberRange, ModelId, OrderId,
    PatternId, VenueId,
};
use jiff::Timestamp;
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftCriteria {
    pub models: Vec<ModelId>,
    pub backdrops: Vec<BackdropId>,
    pub patterns: Vec<PatternId>,
    pub number: Option<GiftNumberRange>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftSide {
    #[default]
    Ask,
    Bid,
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
