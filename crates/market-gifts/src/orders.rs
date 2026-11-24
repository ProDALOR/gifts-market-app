//! Order targets and exact market quotes.
use crate::{
    Amount, BackdropId, CollectibleGift, CollectionId, CurrencyId, GiftId, GiftNumberRange,
    InvalidValue, ModelId, OrderId, PatternId, VenueId,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Eligible ask with concrete trait hints, suitable for read-only market calculations.
#[derive(Debug, Clone)]
pub struct AskQuote {
    /// Stable collectible identity across venues; broad criteria have no concrete gift.
    pub gift_id: Option<GiftId>,
    pub venue_id: VenueId,
    pub price: Amount,
    pub model_id: Option<ModelId>,
    pub backdrop_id: Option<BackdropId>,
    pub pattern_id: Option<PatternId>,
    pub expires_at: Option<Timestamp>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<GiftNumberRange>,
}
impl GiftCriteria {
    /// True when these conditions accept every gift allowed by the selection.
    /// An omitted selection dimension is unrestricted, not an unknown singleton.
    pub fn covers(&self, selection: &Self) -> bool {
        fn alternatives<T: PartialEq>(accepted: &[T], selected: &[T]) -> bool {
            accepted.is_empty()
                || (!selected.is_empty() && selected.iter().all(|v| accepted.contains(v)))
        }
        alternatives(&self.models, &selection.models)
            && alternatives(&self.backdrops, &selection.backdrops)
            && alternatives(&self.patterns, &selection.patterns)
            && self.number.as_ref().is_none_or(|accepted| {
                let selected = selection.number.as_ref();
                accepted
                    .min
                    .is_none_or(|min| selected.and_then(|r| r.min).is_some_and(|v| v >= min))
                    && accepted
                        .max
                        .is_none_or(|max| selected.and_then(|r| r.max).is_some_and(|v| v <= max))
            })
    }
    /// Unknown gift attributes cannot satisfy a restricted bid dimension.
    pub fn accepts(&self, traits: &crate::GiftTraits, number: Option<crate::GiftNumber>) -> bool {
        fn alternative<T: PartialEq>(accepted: &[T], value: Option<&T>) -> bool {
            accepted.is_empty() || value.is_some_and(|v| accepted.contains(v))
        }
        alternative(&self.models, traits.model.as_ref())
            && alternative(&self.backdrops, traits.backdrop.as_ref())
            && alternative(&self.patterns, traits.pattern.as_ref())
            && self.number.as_ref().is_none_or(|range| {
                range.min.is_none_or(|min| number.is_some_and(|v| v >= min))
                    && range.max.is_none_or(|max| number.is_some_and(|v| v <= max))
            })
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Immutable validated order. Construct from `GiftOrderData` at domain boundaries.
///
/// ```compile_fail
/// fn change(order: &mut market_gifts::GiftOrder) {
///     order.quantity = 2;
/// }
/// ```
/// ```compile_fail
/// fn change(order: &mut market_gifts::GiftOrder) {
///     order.book.collection_id = "other".into();
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GiftOrder(GiftOrderData);

/// Construction data; mutable drafts cannot be passed to market consumers as orders.
#[derive(Debug, Clone, PartialEq)]
pub struct GiftOrderData {
    pub id: OrderId,
    pub book: GiftBook,
    pub kind: GiftOrderKind,
    pub target: GiftTarget,
    pub price: Amount,
    pub quantity: u32,
    pub observed_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub executed_at: Option<Timestamp>,
    pub market_url: Option<String>,
}
impl std::ops::Deref for GiftOrder {
    type Target = GiftOrderData;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl GiftOrder {
    pub fn into_data(self) -> GiftOrderData {
        self.0
    }
}
impl TryFrom<GiftOrderData> for GiftOrder {
    type Error = InvalidValue;
    fn try_from(data: GiftOrderData) -> Result<Self, Self::Error> {
        data.validate()?;
        Ok(Self(data))
    }
}
impl GiftOrderData {
    fn validate(&self) -> Result<(), InvalidValue> {
        if self.quantity == 0 {
            return Err(InvalidValue("gift quantity must be a positive integer"));
        }
        if (self.kind == GiftOrderKind::Executed) != self.executed_at.is_some() {
            return Err(InvalidValue(
                "only executed gift orders must have an execution time",
            ));
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

/// Normalize public market links without admitting scripts, embedded credentials or controls.
pub fn market_link(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let url = url::Url::parse(value).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}
