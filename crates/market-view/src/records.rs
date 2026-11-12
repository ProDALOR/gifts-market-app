use crate::*;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
};
pub type Metadata = BTreeMap<String, String>;
pub fn validate_metadata(v: &Metadata) -> Result<(), ValidationError> {
    if v.len() > 16
        || v.iter()
            .any(|(k, v)| k.is_empty() || k.len() > 64 || v.len() > 2048)
        || v.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() > 4096
    {
        Err(ValidationError("invalid metadata size"))
    } else {
        Ok(())
    }
}
pub mod metadata {
    pub fn deserialize_metadata<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<super::Metadata, D::Error> {
        use serde::Deserialize;
        let v = super::Metadata::deserialize(d)?;
        super::validate_metadata(&v).map_err(serde::de::Error::custom)?;
        Ok(v)
    }
}
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
impl PriceQuantity {
    pub fn validate(&self, identified: bool) -> Result<(), ValidationError> {
        self.price.validate()?;
        if identified && self.quantity > Quantity::ONE {
            Err(ValidationError("identified quantity exceeds one"))
        } else {
            Ok(())
        }
    }
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
impl Item {
    pub fn key(&self) -> RecordKey {
        RecordKey {
            feed_id: self.feed_id.clone(),
            id: RecordId::Item(self.item_id.clone()),
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_attributes(&self.attributes)
    }
}
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
impl Order {
    pub fn side(&self) -> OrderSide {
        match self.kind {
            OrderKind::AskItem { .. } | OrderKind::AskUnits { .. } => OrderSide::Ask,
            _ => OrderSide::Bid,
        }
    }
    pub fn item_id(&self) -> Option<&ItemId> {
        match &self.kind {
            OrderKind::AskItem { item_id } | OrderKind::BidItem { item_id } => Some(item_id),
            _ => None,
        }
    }
    pub fn key(&self) -> RecordKey {
        self.reference().key(self.feed_id.clone())
    }
    pub fn reference(&self) -> OrderRef {
        OrderRef {
            order_id: self.order_id.clone(),
            side: self.side(),
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.amounts.validate(self.item_id().is_some())?;
        validate_metadata(&self.meta)?;
        if self.expires_at.is_some_and(|t| t <= self.observed_at) {
            return Err(ValidationError("expiry precedes observation"));
        }
        match &self.kind {
            OrderKind::AskUnits { attributes } => validate_attributes(attributes),
            OrderKind::BidAttributes { filter } => validate_attribute_filter(filter),
            _ => Ok(()),
        }
    }
    pub fn matches_item(&self, item: &Item) -> bool {
        self.item_type_id == item.item_type_id
            && match &self.kind {
                OrderKind::AskItem { item_id } | OrderKind::BidItem { item_id } => {
                    item_id == &item.item_id
                }
                OrderKind::BidAny => true,
                OrderKind::BidAttributes { filter } => {
                    attribute_filter_matches(filter, &item.attributes)
                }
                OrderKind::AskUnits { .. } => false,
            }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderRef {
    pub order_id: OrderId,
    pub side: OrderSide,
}
impl OrderRef {
    pub fn key(&self, feed_id: FeedId) -> RecordKey {
        RecordKey {
            feed_id,
            id: match self.side {
                OrderSide::Ask => RecordId::Ask(self.order_id.clone()),
                OrderSide::Bid => RecordId::Bid(self.order_id.clone()),
            },
        }
    }
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
impl Record {
    pub fn key(&self) -> RecordKey {
        match self {
            Self::Item(v) => v.key(),
            Self::Order(v) => v.key(),
            Self::Feed(v) => RecordKey {
                feed_id: v.feed_id.clone(),
                id: RecordId::Feed(v.feed_id.clone()),
            },
        }
    }
    pub fn item_reference(&self) -> Option<(&ItemId, &ItemTypeId)> {
        match self {
            Self::Item(v) => Some((&v.item_id, &v.item_type_id)),
            Self::Order(v) => v.item_id().map(|id| (id, &v.item_type_id)),
            _ => None,
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Item(v) => v.validate(),
            Self::Order(v) => v.validate(),
            Self::Feed(_) => Ok(()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Mutation {
    Upsert(Record),
    Delete(RecordKey),
}
impl Mutation {
    pub fn key(&self) -> RecordKey {
        match self {
            Self::Upsert(v) => v.key(),
            Self::Delete(v) => v.clone(),
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub generation: String,
    #[serde(with = "crate::numbers::u64_string")]
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeBatch {
    pub cursor: Cursor,
    pub catalog_revision: String,
    pub catalog: Vec<CatalogEntry>,
    pub events: Vec<MarketEvent>,
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
impl Trade {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.amounts
            .validate(matches!(self.kind, TradeKind::Item { .. }))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SourceEvent {
    OrderStatus(OrderStatusEvent),
    Trade(Trade),
}
impl SourceEvent {
    pub fn feed_id(&self) -> &FeedId {
        match self {
            Self::OrderStatus(v) => &v.feed_id,
            Self::Trade(v) => &v.feed_id,
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::OrderStatus(_) => Ok(()),
            Self::Trade(v) => v.validate(),
        }
    }
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
impl MarketEvent {
    pub fn mutation(&self) -> Option<Mutation> {
        match self {
            Self::Upsert(v) => Some(Mutation::Upsert(v.clone())),
            Self::Delete(v) => Some(Mutation::Delete(v.clone())),
            Self::OrderRemoved(v) => Some(Mutation::Delete(v.data.key(v.feed_id.clone()))),
            _ => None,
        }
    }
}
impl Record {
    pub fn item_type_id(&self) -> Option<&ItemTypeId> {
        match self {
            Self::Item(v) => Some(&v.item_type_id),
            Self::Order(v) => Some(&v.item_type_id),
            Self::Feed(_) => None,
        }
    }
}
impl MarketEvent {
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            Self::Upsert(_) | Self::Delete(_) | Self::OrderRemoved(_)
        )
    }
    pub fn is_history(&self) -> bool {
        matches!(
            self,
            Self::Upsert(Record::Order(_))
                | Self::OrderRemoved(_)
                | Self::OrderStatus(_)
                | Self::Trade(_)
        )
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Upsert(v) => v.validate(),
            Self::Delete(v) => v.validate(),
            Self::Trade(v) => v.validate(),
            _ => Ok(()),
        }
    }
}
impl From<SourceEvent> for MarketEvent {
    fn from(v: SourceEvent) -> Self {
        match v {
            SourceEvent::Trade(v) => Self::Trade(v),
            SourceEvent::OrderStatus(v) => Self::OrderStatus(v),
        }
    }
}
