use super::wire as w;
use crate::*;
use std::collections::{BTreeMap, BTreeSet};
fn required<T>(v: Option<T>) -> Result<T, ValidationError> {
    v.ok_or(ValidationError("missing protocol field"))
}
fn invalid() -> ValidationError {
    ValidationError("unknown protocol variant")
}
impl From<Cursor> for w::Cursor {
    fn from(v: Cursor) -> Self {
        Self {
            generation: v.generation,
            revision: v.revision,
        }
    }
}
impl From<w::Cursor> for Cursor {
    fn from(v: w::Cursor) -> Self {
        Self {
            generation: v.generation,
            revision: v.revision,
        }
    }
}
macro_rules! enums{($local:ty,$wire:ty, $($variant:ident),+)=>{
impl From<$local> for $wire{fn from(v:$local)->Self{match v{$(<$local>::$variant=>Self::$variant),+}}}
impl TryFrom<$wire> for $local{type Error=ValidationError;fn try_from(v:$wire)->Result<Self,Self::Error>{match v{$(<$wire>::$variant=>Ok(Self::$variant),)+_=>Err(invalid())}}}
};}
enums!(RecordKind, w::RecordKind, Item, Ask, Bid, Feed);
enums!(FeedState, w::FeedState, Syncing, Ready, Stale, Unavailable);
enums!(OrderSide, w::OrderSide, Ask, Bid);
enums!(
    OrderStatus,
    w::OrderStatus,
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Expired
);
enums!(
    AttributeType,
    w::AttributeType,
    Enum,
    Integer,
    Decimal,
    Boolean,
    Text,
    Timestamp
);
impl From<Money> for w::Money {
    fn from(v: Money) -> Self {
        Self {
            currency_id: v.currency_id.into(),
            amount: v.amount.to_string(),
        }
    }
}
impl TryFrom<w::Money> for Money {
    type Error = ValidationError;
    fn try_from(v: w::Money) -> Result<Self, Self::Error> {
        Self::parse(v.currency_id.try_into()?, &v.amount)
    }
}
impl From<PriceQuantity> for w::PriceQuantity {
    fn from(v: PriceQuantity) -> Self {
        Self {
            price: Some(v.price.into()),
            quantity: v.quantity.to_string(),
        }
    }
}
impl TryFrom<w::PriceQuantity> for PriceQuantity {
    type Error = ValidationError;
    fn try_from(v: w::PriceQuantity) -> Result<Self, Self::Error> {
        Ok(Self {
            price: required(v.price)?.try_into()?,
            quantity: v.quantity.parse()?,
        })
    }
}
impl From<RecordKey> for w::RecordKey {
    fn from(v: RecordKey) -> Self {
        Self {
            kind: w::RecordKind::from(v.kind()) as i32,
            feed_id: v.feed_id.into(),
            id: match v.id {
                RecordId::Item(v) => v.into(),
                RecordId::Ask(v) | RecordId::Bid(v) => v.into(),
                RecordId::Feed(v) => v.into(),
            },
        }
    }
}
impl TryFrom<w::RecordKey> for RecordKey {
    type Error = ValidationError;
    fn try_from(v: w::RecordKey) -> Result<Self, Self::Error> {
        let feed_id = v.feed_id.try_into()?;
        let id = match w::RecordKind::try_from(v.kind).map_err(|_| invalid())? {
            w::RecordKind::Item => RecordId::Item(v.id.try_into()?),
            w::RecordKind::Ask => RecordId::Ask(v.id.try_into()?),
            w::RecordKind::Bid => RecordId::Bid(v.id.try_into()?),
            w::RecordKind::Feed => RecordId::Feed(v.id.try_into()?),
            _ => return Err(invalid()),
        };
        let key = Self { feed_id, id };
        key.validate()?;
        Ok(key)
    }
}
impl From<AttributeValue> for w::AttributeValue {
    fn from(v: AttributeValue) -> Self {
        use w::attribute_value::Value as V;
        Self {
            value: Some(match v {
                AttributeValue::Enum(v) => V::EnumId(v.into()),
                AttributeValue::Integer(v) => V::Integer(v),
                AttributeValue::Decimal(v) => V::Decimal(v.to_string()),
                AttributeValue::Boolean(v) => V::Boolean(v),
                AttributeValue::Text(v) => V::Text(v),
                AttributeValue::Timestamp(v) => V::TimestampMs(v.as_millis()),
            }),
        }
    }
}
impl TryFrom<w::AttributeValue> for AttributeValue {
    type Error = ValidationError;
    fn try_from(v: w::AttributeValue) -> Result<Self, Self::Error> {
        use w::attribute_value::Value as V;
        let value = match required(v.value)? {
            V::EnumId(v) => Self::Enum(v.try_into()?),
            V::Integer(v) => Self::Integer(v),
            V::Decimal(v) => Self::decimal(&v)?,
            V::Boolean(v) => Self::Boolean(v),
            V::Text(v) => Self::Text(v),
            V::TimestampMs(v) => Self::Timestamp(v.try_into()?),
        };
        value.validate()?;
        Ok(value)
    }
}
fn attrs(v: Attributes) -> BTreeMap<String, w::AttributeValue> {
    v.into_iter().map(|(k, v)| (k.into(), v.into())).collect()
}
fn read_attrs(v: BTreeMap<String, w::AttributeValue>) -> Result<Attributes, ValidationError> {
    v.into_iter()
        .map(|(k, v)| Ok((k.try_into()?, v.try_into()?)))
        .collect()
}
impl From<AttributePredicate> for w::AttributePredicate {
    fn from(v: AttributePredicate) -> Self {
        use w::attribute_predicate::Predicate as P;
        Self {
            predicate: Some(match v {
                AttributePredicate::Eq { value } => P::Eq(value.into()),
                AttributePredicate::OneOf { values } => P::OneOf(w::AttributeValues {
                    values: values.into_iter().map(Into::into).collect(),
                }),
                AttributePredicate::Range { min, max } => P::Range(w::AttributeRange {
                    min: min.map(Into::into),
                    max: max.map(Into::into),
                }),
            }),
        }
    }
}
impl TryFrom<w::AttributePredicate> for AttributePredicate {
    type Error = ValidationError;
    fn try_from(v: w::AttributePredicate) -> Result<Self, Self::Error> {
        use w::attribute_predicate::Predicate as P;
        let value = match required(v.predicate)? {
            P::Eq(v) => Self::Eq {
                value: v.try_into()?,
            },
            P::OneOf(v) => {
                let n = v.values.len();
                let values: BTreeSet<_> = v
                    .values
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_, _>>()?;
                if values.len() != n {
                    return Err(ValidationError("duplicate predicate value"));
                }
                Self::OneOf { values }
            }
            P::Range(v) => Self::Range {
                min: v.min.map(TryInto::try_into).transpose()?,
                max: v.max.map(TryInto::try_into).transpose()?,
            },
        };
        value.validate()?;
        Ok(value)
    }
}
fn filter(v: AttributeFilter) -> w::AttributeFilter {
    w::AttributeFilter {
        attributes: v.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
    }
}
fn read_filter(v: w::AttributeFilter) -> Result<AttributeFilter, ValidationError> {
    v.attributes
        .into_iter()
        .map(|(k, v)| Ok((k.try_into()?, v.try_into()?)))
        .collect()
}
impl From<FeedStatus> for w::FeedStatus {
    fn from(v: FeedStatus) -> Self {
        Self {
            feed_id: v.feed_id.into(),
            venue_id: v.venue_id.into(),
            state: w::FeedState::from(v.state) as i32,
            last_sync_ms: v.last_sync_at.map(Into::into),
        }
    }
}
impl TryFrom<w::FeedStatus> for FeedStatus {
    type Error = ValidationError;
    fn try_from(v: w::FeedStatus) -> Result<Self, Self::Error> {
        Ok(Self {
            feed_id: v.feed_id.try_into()?,
            venue_id: v.venue_id.try_into()?,
            state: w::FeedState::try_from(v.state)
                .map_err(|_| invalid())?
                .try_into()?,
            last_sync_at: v.last_sync_ms.map(TryInto::try_into).transpose()?,
        })
    }
}
impl From<ItemData> for w::ItemData {
    fn from(v: ItemData) -> Self {
        Self {
            item_id: v.item_id.into(),
            attributes: attrs(v.attributes),
        }
    }
}
impl TryFrom<w::ItemData> for ItemData {
    type Error = ValidationError;
    fn try_from(v: w::ItemData) -> Result<Self, Self::Error> {
        Ok(Self {
            item_id: v.item_id.try_into()?,
            attributes: read_attrs(v.attributes)?,
        })
    }
}
impl From<OrderData> for w::OrderData {
    fn from(v: OrderData) -> Self {
        use w::order_data::Kind as K;
        Self {
            order_id: v.order_id.into(),
            amounts: Some(v.amounts.into()),
            expires_at_ms: v.expires_at.map(Into::into),
            meta: v.meta,
            kind: Some(match v.kind {
                OrderKind::AskItem { item_id } => K::AskItemId(item_id.into()),
                OrderKind::AskUnits { attributes } => K::AskUnits(w::AttributeMap {
                    attributes: attrs(attributes),
                }),
                OrderKind::BidItem { item_id } => K::BidItemId(item_id.into()),
                OrderKind::BidAny => K::BidAny(w::Empty {}),
                OrderKind::BidAttributes { filter: v } => K::BidAttributes(filter(v)),
            }),
        }
    }
}
impl TryFrom<w::OrderData> for OrderData {
    type Error = ValidationError;
    fn try_from(v: w::OrderData) -> Result<Self, Self::Error> {
        use w::order_data::Kind as K;
        Ok(Self {
            order_id: v.order_id.try_into()?,
            amounts: required(v.amounts)?.try_into()?,
            expires_at: v.expires_at_ms.map(TryInto::try_into).transpose()?,
            meta: v.meta,
            kind: match required(v.kind)? {
                K::AskItemId(v) => OrderKind::AskItem {
                    item_id: v.try_into()?,
                },
                K::AskUnits(v) => OrderKind::AskUnits {
                    attributes: read_attrs(v.attributes)?,
                },
                K::BidItemId(v) => OrderKind::BidItem {
                    item_id: v.try_into()?,
                },
                K::BidAny(_) => OrderKind::BidAny,
                K::BidAttributes(v) => OrderKind::BidAttributes {
                    filter: read_filter(v)?,
                },
            },
        })
    }
}
impl From<OrderRef> for w::OrderRef {
    fn from(v: OrderRef) -> Self {
        Self {
            order_id: v.order_id.into(),
            side: w::OrderSide::from(v.side) as i32,
        }
    }
}
impl TryFrom<w::OrderRef> for OrderRef {
    type Error = ValidationError;
    fn try_from(v: w::OrderRef) -> Result<Self, Self::Error> {
        Ok(Self {
            order_id: v.order_id.try_into()?,
            side: w::OrderSide::try_from(v.side)
                .map_err(|_| invalid())?
                .try_into()?,
        })
    }
}
impl From<OrderStatusData> for w::OrderStatusData {
    fn from(v: OrderStatusData) -> Self {
        Self {
            event_id: v.event_id.into(),
            order: Some(v.order.into()),
            status: w::OrderStatus::from(v.status) as i32,
            occurred_at_ms: v.occurred_at.map(Into::into),
        }
    }
}
impl TryFrom<w::OrderStatusData> for OrderStatusData {
    type Error = ValidationError;
    fn try_from(v: w::OrderStatusData) -> Result<Self, Self::Error> {
        Ok(Self {
            event_id: v.event_id.try_into()?,
            order: required(v.order)?.try_into()?,
            status: w::OrderStatus::try_from(v.status)
                .map_err(|_| invalid())?
                .try_into()?,
            occurred_at: v.occurred_at_ms.map(TryInto::try_into).transpose()?,
        })
    }
}
impl From<TradeData> for w::TradeData {
    fn from(v: TradeData) -> Self {
        Self {
            trade_id: v.trade_id.into(),
            amounts: Some(v.amounts.into()),
            executed_at_ms: v.executed_at.into(),
            buy_order_id: v.buy_order_id.map(Into::into),
            sell_order_id: v.sell_order_id.map(Into::into),
            kind: Some(match v.kind {
                TradeKind::Item { item_id } => w::trade_data::Kind::ItemId(item_id.into()),
                TradeKind::Units { attributes } => w::trade_data::Kind::Units(w::AttributeMap {
                    attributes: attrs(attributes),
                }),
            }),
        }
    }
}
impl TryFrom<w::TradeData> for TradeData {
    type Error = ValidationError;
    fn try_from(v: w::TradeData) -> Result<Self, Self::Error> {
        Ok(Self {
            trade_id: v.trade_id.try_into()?,
            amounts: required(v.amounts)?.try_into()?,
            executed_at: v.executed_at_ms.try_into()?,
            buy_order_id: v.buy_order_id.map(TryInto::try_into).transpose()?,
            sell_order_id: v.sell_order_id.map(TryInto::try_into).transpose()?,
            kind: match required(v.kind)? {
                w::trade_data::Kind::ItemId(v) => TradeKind::Item {
                    item_id: v.try_into()?,
                },
                w::trade_data::Kind::Units(v) => TradeKind::Units {
                    attributes: read_attrs(v.attributes)?,
                },
            },
        })
    }
}
macro_rules! observed {
    ($payload:ty,$wire:ty) => {
        impl From<Observed<$payload>> for $wire {
            fn from(v: Observed<$payload>) -> Self {
                Self {
                    observation: Some(w::Observation {
                        feed_id: v.feed_id.into(),
                        item_type_id: v.item_type_id.into(),
                        observed_at_ms: v.observed_at.into(),
                    }),
                    data: Some(v.data.into()),
                }
            }
        }
        impl TryFrom<$wire> for Observed<$payload> {
            type Error = ValidationError;
            fn try_from(v: $wire) -> Result<Self, Self::Error> {
                let o = required(v.observation)?;
                Ok(Self {
                    feed_id: o.feed_id.try_into()?,
                    item_type_id: o.item_type_id.try_into()?,
                    observed_at: o.observed_at_ms.try_into()?,
                    data: required(v.data)?.try_into()?,
                })
            }
        }
    };
}
observed!(ItemData, w::Item);
observed!(OrderData, w::Order);
observed!(TradeData, w::Trade);
observed!(OrderStatusData, w::OrderStatusEvent);
observed!(OrderRef, w::OrderRemoval);
impl From<Record> for w::Record {
    fn from(v: Record) -> Self {
        Self {
            record: Some(match v {
                Record::Item(v) => w::record::Record::Item(v.into()),
                Record::Order(v) => w::record::Record::Order(v.into()),
                Record::Feed(v) => w::record::Record::Feed(v.into()),
            }),
        }
    }
}
impl TryFrom<w::Record> for Record {
    type Error = ValidationError;
    fn try_from(v: w::Record) -> Result<Self, Self::Error> {
        Ok(match required(v.record)? {
            w::record::Record::Item(v) => Self::Item(v.try_into()?),
            w::record::Record::Order(v) => Self::Order(v.try_into()?),
            w::record::Record::Feed(v) => Self::Feed(v.try_into()?),
        })
    }
}
impl From<Mutation> for w::Mutation {
    fn from(v: Mutation) -> Self {
        Self {
            mutation: Some(match v {
                Mutation::Upsert(v) => w::mutation::Mutation::Upsert(v.into()),
                Mutation::Delete(v) => w::mutation::Mutation::Delete(v.into()),
            }),
        }
    }
}
impl TryFrom<w::Mutation> for Mutation {
    type Error = ValidationError;
    fn try_from(v: w::Mutation) -> Result<Self, Self::Error> {
        Ok(match required(v.mutation)? {
            w::mutation::Mutation::Upsert(v) => Self::Upsert(v.try_into()?),
            w::mutation::Mutation::Delete(v) => Self::Delete(v.try_into()?),
        })
    }
}
impl From<SourceEvent> for w::SourceEvent {
    fn from(v: SourceEvent) -> Self {
        Self {
            event: Some(match v {
                SourceEvent::OrderStatus(v) => w::source_event::Event::OrderStatus(v.into()),
                SourceEvent::Trade(v) => w::source_event::Event::Trade(v.into()),
            }),
        }
    }
}
impl TryFrom<w::SourceEvent> for SourceEvent {
    type Error = ValidationError;
    fn try_from(v: w::SourceEvent) -> Result<Self, Self::Error> {
        Ok(match required(v.event)? {
            w::source_event::Event::OrderStatus(v) => Self::OrderStatus(v.try_into()?),
            w::source_event::Event::Trade(v) => Self::Trade(v.try_into()?),
        })
    }
}
impl From<MarketEvent> for w::MarketEvent {
    fn from(v: MarketEvent) -> Self {
        Self {
            event: Some(match v {
                MarketEvent::Upsert(v) => w::market_event::Event::Upsert(v.into()),
                MarketEvent::Delete(v) => w::market_event::Event::Delete(v.into()),
                MarketEvent::OrderRemoved(v) => w::market_event::Event::OrderRemoved(v.into()),
                MarketEvent::OrderStatus(v) => w::market_event::Event::OrderStatus(v.into()),
                MarketEvent::Trade(v) => w::market_event::Event::Trade(v.into()),
            }),
        }
    }
}
impl TryFrom<w::MarketEvent> for MarketEvent {
    type Error = ValidationError;
    fn try_from(v: w::MarketEvent) -> Result<Self, Self::Error> {
        Ok(match required(v.event)? {
            w::market_event::Event::Upsert(v) => Self::Upsert(v.try_into()?),
            w::market_event::Event::Delete(v) => Self::Delete(v.try_into()?),
            w::market_event::Event::OrderRemoved(v) => Self::OrderRemoved(v.try_into()?),
            w::market_event::Event::OrderStatus(v) => Self::OrderStatus(v.try_into()?),
            w::market_event::Event::Trade(v) => Self::Trade(v.try_into()?),
        })
    }
}
impl From<ExternalId> for w::ExternalId {
    fn from(v: ExternalId) -> Self {
        Self {
            namespace: v.namespace,
            id: v.id,
        }
    }
}
impl From<w::ExternalId> for ExternalId {
    fn from(v: w::ExternalId) -> Self {
        Self {
            namespace: v.namespace,
            id: v.id,
        }
    }
}
impl From<CatalogEntry> for w::CatalogEntry {
    fn from(v: CatalogEntry) -> Self {
        use w::catalog_entry::Entry as E;
        Self {
            entry: Some(match v {
                CatalogEntry::Venue(v) => E::Venue(w::VenueDefinition {
                    id: v.id.into(),
                    name: v.name,
                }),
                CatalogEntry::Currency(v) => E::Currency(w::CurrencyDefinition {
                    id: v.id.into(),
                    name: v.name,
                    decimals: v.decimals,
                }),
                CatalogEntry::ItemType(v) => E::ItemType(w::ItemTypeDefinition {
                    id: v.id.into(),
                    name: v.name,
                    quantity_step: v.quantity_step.to_string(),
                    aliases: v.aliases.into_iter().map(Into::into).collect(),
                    attributes: v
                        .attributes
                        .into_iter()
                        .map(|(k, v)| {
                            (
                                k.into(),
                                w::AttributeRule {
                                    allowed_values: v.allowed_values.map(|v| {
                                        w::AllowedAttributeValues {
                                            ids: v.into_iter().map(Into::into).collect(),
                                        }
                                    }),
                                },
                            )
                        })
                        .collect(),
                }),
                CatalogEntry::Attribute(v) => E::Attribute(w::AttributeDefinition {
                    id: v.id.into(),
                    name: v.name,
                    value_type: w::AttributeType::from(v.value_type) as i32,
                    minimum: v.minimum.map(Into::into),
                    maximum: v.maximum.map(Into::into),
                    max_length: v.max_length.map(|v| v as u32),
                }),
                CatalogEntry::AttributeValue(v) => E::AttributeValue(w::AttributeValueDefinition {
                    id: v.id.into(),
                    attribute_id: v.attribute_id.into(),
                    name: v.name,
                    aliases: v.aliases.into_iter().map(Into::into).collect(),
                    meta: v.meta,
                }),
                CatalogEntry::Feed(v) => E::Feed(w::FeedDefinition {
                    id: v.id.into(),
                    venue_id: v.venue_id.into(),
                }),
            }),
        }
    }
}
impl TryFrom<w::CatalogEntry> for CatalogEntry {
    type Error = ValidationError;
    fn try_from(v: w::CatalogEntry) -> Result<Self, Self::Error> {
        use w::catalog_entry::Entry as E;
        Ok(match required(v.entry)? {
            E::Venue(v) => Self::Venue(VenueDefinition {
                id: v.id.try_into()?,
                name: v.name,
            }),
            E::Currency(v) => Self::Currency(CurrencyDefinition {
                id: v.id.try_into()?,
                name: v.name,
                decimals: v.decimals,
            }),
            E::ItemType(v) => Self::ItemType(ItemTypeDefinition {
                id: v.id.try_into()?,
                name: v.name,
                quantity_step: v.quantity_step.parse()?,
                aliases: v.aliases.into_iter().map(Into::into).collect(),
                attributes: v
                    .attributes
                    .into_iter()
                    .map(|(k, v)| {
                        Ok((
                            k.try_into()?,
                            AttributeRule {
                                allowed_values: v
                                    .allowed_values
                                    .map(|v| {
                                        v.ids.into_iter().map(TryInto::try_into).collect::<Result<
                                            BTreeSet<_>,
                                            _,
                                        >>(
                                        )
                                    })
                                    .transpose()?,
                            },
                        ))
                    })
                    .collect::<Result<_, ValidationError>>()?,
            }),
            E::Attribute(v) => Self::Attribute(AttributeDefinition {
                id: v.id.try_into()?,
                name: v.name,
                value_type: w::AttributeType::try_from(v.value_type)
                    .map_err(|_| invalid())?
                    .try_into()?,
                minimum: v.minimum.map(TryInto::try_into).transpose()?,
                maximum: v.maximum.map(TryInto::try_into).transpose()?,
                max_length: v.max_length.map(|v| v as usize),
            }),
            E::AttributeValue(v) => Self::AttributeValue(AttributeValueDefinition {
                id: v.id.try_into()?,
                attribute_id: v.attribute_id.try_into()?,
                name: v.name,
                aliases: v.aliases.into_iter().map(Into::into).collect(),
                meta: v.meta,
            }),
            E::Feed(v) => Self::Feed(FeedDefinition {
                id: v.id.try_into()?,
                venue_id: v.venue_id.try_into()?,
            }),
        })
    }
}
