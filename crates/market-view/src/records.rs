use crate::*;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
};
pub type Metadata = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observed<T> {
    pub feed_id: FeedId,
    pub item_type_id: ItemTypeId,
    pub observed_at: MarketTimestamp,
    pub data: T,
}
impl<T> Deref for Observed<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.data
    }
}
impl<T> DerefMut for Observed<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.data
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub currency_id: CurrencyId,
    #[serde(with = "crate::primitives::decimal")]
    pub amount: Decimal,
}
impl Money {
    pub fn parse(currency_id: CurrencyId, s: &str) -> Result<Self, ValidationError> {
        let m = Self {
            currency_id,
            amount: Decimal::from_str_exact(s).map_err(|_| ValidationError("invalid price"))?,
        };
        m.validate()?;
        Ok(m)
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.amount <= Decimal::ZERO {
            Err(ValidationError("price must be positive"))
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceQuantity {
    pub price: Money,
    pub quantity: Quantity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Item,
    Ask,
    Bid,
    Feed,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum RecordId {
    Item(ItemId),
    Ask(OrderId),
    Bid(OrderId),
    Feed(FeedId),
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RecordKey {
    pub feed_id: FeedId,
    #[serde(flatten)]
    pub id: RecordId,
}
impl RecordKey {
    pub fn kind(&self) -> RecordKind {
        match self.id {
            RecordId::Item(_) => RecordKind::Item,
            RecordId::Ask(_) => RecordKind::Ask,
            RecordId::Bid(_) => RecordKind::Bid,
            RecordId::Feed(_) => RecordKind::Feed,
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        if matches!(&self.id,RecordId::Feed(id) if id!=&self.feed_id) {
            Err(ValidationError("feed key differs"))
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemData {
    pub item_id: ItemId,
    #[serde(deserialize_with = "crate::attributes::deserialize_attribute_map")]
    pub attributes: Attributes,
}
pub type Item = Observed<ItemData>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSide {
    Ask,
    Bid,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OrderKind {
    AskItem {
        item_id: ItemId,
    },
    AskUnits {
        #[serde(deserialize_with = "crate::attributes::deserialize_attribute_map")]
        attributes: Attributes,
    },
    BidItem {
        item_id: ItemId,
    },
    BidAny,
    BidAttributes {
        #[serde(deserialize_with = "crate::attributes::deserialize_attribute_map")]
        filter: AttributeFilter,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderData {
    pub order_id: OrderId,
    pub amounts: PriceQuantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<MarketTimestamp>,
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    pub meta: Metadata,
    pub kind: OrderKind,
}
pub type Order = Observed<OrderData>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderRef {
    pub order_id: OrderId,
    pub side: OrderSide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedState {
    Syncing,
    Ready,
    Stale,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedStatus {
    pub feed_id: FeedId,
    pub venue_id: VenueId,
    pub state: FeedState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<MarketTimestamp>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Record {
    Item(Item),
    Order(Order),
    Feed(FeedStatus),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub generation: String,
    #[serde(with = "crate::numbers::u64_string")]
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Expired,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderStatusData {
    pub event_id: OrderEventId,
    pub order: OrderRef,
    pub status: OrderStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<MarketTimestamp>,
}
pub type OrderStatusEvent = Observed<OrderStatusData>;
pub type OrderRemoval = Observed<OrderRef>;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TradeKind {
    Item { item_id: ItemId },
    Units { attributes: Attributes },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradeData {
    pub trade_id: TradeId,
    pub amounts: PriceQuantity,
    pub executed_at: MarketTimestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buy_order_id: Option<OrderId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sell_order_id: Option<OrderId>,
    pub kind: TradeKind,
}
pub type Trade = Observed<TradeData>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SourceEvent {
    OrderStatus(OrderStatusEvent),
    Trade(Trade),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum MarketEvent {
    Upsert(Record),
    Delete(RecordKey),
    OrderRemoved(OrderRemoval),
    OrderStatus(OrderStatusEvent),
    Trade(Trade),
}

impl From<SourceEvent> for MarketEvent {
    fn from(v: SourceEvent) -> Self {
        match v {
            SourceEvent::Trade(v) => Self::Trade(v),
            SourceEvent::OrderStatus(v) => Self::OrderStatus(v),
        }
    }
}
