//! Order targets and exact market quotes.
use crate::{
    Amount, BackdropId, CollectibleGift, CollectionId, CurrencyId, GiftNumberRange, InvalidValue,
    ModelId, OrderId, PatternId, VenueId,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Eligible ask with concrete trait hints, suitable for read-only market calculations.
#[derive(Debug, Clone)]
pub struct AskQuote {
    pub venue_id: VenueId,
    pub price: Amount,
    pub model_id: Option<ModelId>,
    pub backdrop_id: Option<BackdropId>,
}

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

/// Stable reasons an order target cannot be represented as gift facts or criteria.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum UnsupportedGiftReason {
    UnsupportedConstraints,
    InvalidAttributes,
    NonConcreteGift,
    InvalidGiftId,
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
        reason: UnsupportedGiftReason,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftOrder {
    pub id: OrderId,
    pub book: GiftBook,
    pub kind: GiftOrderKind,
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

impl GiftOrder {
    /// Invariants shared by hub projection and deserialization. Unknown traits remain valid.
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if self.quantity == 0 {
            return Err(InvalidValue("gift quantity must be a positive integer"));
        }
        match &self.target {
            GiftTarget::Gift { gift } => {
                if self.quantity != 1 {
                    return Err(InvalidValue("a concrete gift must have quantity one"));
                }
                if gift.collection_id != self.book.collection_id {
                    return Err(InvalidValue("gift collection must match the order book"));
                }
            }
            GiftTarget::Criteria { criteria } => {
                if let Some(range) = &criteria.number {
                    range.validate()?;
                }
            }
            GiftTarget::Unsupported { .. } => {}
        }
        Ok(())
    }
}
// Remote derive supplies field parsing once; both entry points use the same validation.
impl<'de> Deserialize<'de> for GiftOrder {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let order = Self::deserialize(deserializer)?;
        order.validate().map_err(serde::de::Error::custom)?;
        Ok(order)
    }
}
impl Serialize for GiftOrder {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        Self::serialize(self, serializer)
    }
}
