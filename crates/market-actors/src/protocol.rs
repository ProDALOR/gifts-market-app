//! Private adapters between actor contracts and replica/hub representations.
use super::{
    error::{DataError, Result},
    models::*,
};
use jiff::Timestamp;
use market_hub_protocol::wire as w;
use std::collections::BTreeMap;
fn missing() -> DataError {
    DataError::unavailable("invalid market response")
}
fn required<T>(v: Option<T>) -> Result<T> {
    v.ok_or_else(missing)
}
fn time(ms: i64) -> Result<Timestamp> {
    Timestamp::from_millisecond(ms).map_err(|_| missing())
}
impl From<BookKey> for w::BookKey {
    fn from(v: BookKey) -> Self {
        Self {
            venue_id: v.venue_id,
            item_type_id: v.item_type_id,
            currency_id: v.currency_id,
        }
    }
}
impl From<w::BookKey> for BookKey {
    fn from(v: w::BookKey) -> Self {
        Self {
            venue_id: v.venue_id,
            item_type_id: v.item_type_id,
            currency_id: v.currency_id,
        }
    }
}
impl From<Mode> for w::OrderMode {
    fn from(v: Mode) -> Self {
        match v {
            Mode::Ask => Self::Ask,
            Mode::Bid => Self::Bid,
            Mode::Executed => Self::Executed,
        }
    }
}
impl From<Sort> for w::ReadSort {
    fn from(v: Sort) -> Self {
        match v {
            Sort::Identity => Self::Identity,
            Sort::PriceAsc => Self::PriceAsc,
            Sort::PriceDesc => Self::PriceDesc,
            Sort::TimeAsc => Self::TimeAsc,
            Sort::TimeDesc => Self::TimeDesc,
            Sort::NumberAsc => Self::NumberAsc,
            Sort::NumberDesc => Self::NumberDesc,
        }
    }
}
impl TryFrom<AttributeValue> for w::AttributeValue {
    type Error = DataError;
    fn try_from(v: AttributeValue) -> Result<Self> {
        use w::attribute_value::Value as V;
        Ok(Self {
            value: Some(match v {
                AttributeValue::Enum { value: v } => V::OptionId(v),
                AttributeValue::Integer { value: v } => V::Integer(
                    v.parse()
                        .map_err(|_| DataError::invalid("invalid integer attribute"))?,
                ),
                AttributeValue::Decimal { value: v } => V::Decimal(v),
                AttributeValue::Boolean { value: v } => V::Boolean(v),
                AttributeValue::Text { value: v } => V::Text(v),
                AttributeValue::Timestamp { value: v } => V::TimestampMs(v.as_millisecond()),
                AttributeValue::List { value: v } => V::List(w::AttributeValues {
                    values: v
                        .into_iter()
                        .map(TryInto::try_into)
                        .collect::<Result<_>>()?,
                }),
                AttributeValue::Object { value: v } => V::Object(w::AttributeObject {
                    fields: v
                        .into_iter()
                        .map(|(k, v)| Ok((k, v.try_into()?)))
                        .collect::<Result<_>>()?,
                }),
            }),
        })
    }
}
impl TryFrom<w::AttributeValue> for AttributeValue {
    type Error = DataError;
    fn try_from(v: w::AttributeValue) -> Result<Self> {
        use w::attribute_value::Value as V;
        Ok(match required(v.value)? {
            V::OptionId(v) => Self::Enum { value: v },
            V::Integer(v) => Self::Integer {
                value: v.to_string(),
            },
            V::Decimal(v) => Self::Decimal { value: v },
            V::Boolean(v) => Self::Boolean { value: v },
            V::Text(v) => Self::Text { value: v },
            V::TimestampMs(v) => Self::Timestamp { value: time(v)? },
            V::List(v) => Self::List {
                value: v
                    .values
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_>>()?,
            },
            V::Object(v) => Self::Object {
                value: v
                    .fields
                    .into_iter()
                    .map(|(k, v)| Ok((k, v.try_into()?)))
                    .collect::<Result<_>>()?,
            },
        })
    }
}
fn encode(v: BTreeMap<String, Predicate>) -> Result<BTreeMap<String, w::AttributePredicate>> {
    v.into_iter().map(|(k, v)| Ok((k, v.try_into()?))).collect()
}
fn decode(v: BTreeMap<String, w::AttributePredicate>) -> Result<BTreeMap<String, Predicate>> {
    v.into_iter().map(|(k, v)| Ok((k, v.try_into()?))).collect()
}
impl TryFrom<Predicate> for w::AttributePredicate {
    type Error = DataError;
    fn try_from(v: Predicate) -> Result<Self> {
        use w::attribute_predicate::Predicate as P;
        Ok(Self {
            predicate: Some(match v {
                Predicate::Eq { value } => P::Eq(value.try_into()?),
                Predicate::OneOf { values } => P::OneOf(w::AttributeValues {
                    values: values
                        .into_iter()
                        .map(TryInto::try_into)
                        .collect::<Result<_>>()?,
                }),
                Predicate::Range { min, max } => P::Range(w::AttributeRange {
                    min: min.map(TryInto::try_into).transpose()?,
                    max: max.map(TryInto::try_into).transpose()?,
                }),
                Predicate::Object { fields } => P::Object(w::ObjectPredicate {
                    fields: encode(fields)?,
                }),
                Predicate::Contains {
                    predicate,
                    min_count,
                    max_count,
                } => P::Contains(Box::new(w::ContainsPredicate {
                    predicate: Some(Box::new((*predicate).try_into()?)),
                    min_count,
                    max_count,
                })),
                Predicate::AllOf { predicates } => P::AllOf(w::AttributePredicates {
                    predicates: predicates
                        .into_iter()
                        .map(TryInto::try_into)
                        .collect::<Result<_>>()?,
                }),
            }),
        })
    }
}
impl TryFrom<w::AttributePredicate> for Predicate {
    type Error = DataError;
    fn try_from(v: w::AttributePredicate) -> Result<Self> {
        use w::attribute_predicate::Predicate as P;
        Ok(match required(v.predicate)? {
            P::Eq(value) => Self::Eq {
                value: value.try_into()?,
            },
            P::OneOf(v) => Self::OneOf {
                values: v
                    .values
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_>>()?,
            },
            P::Range(v) => Self::Range {
                min: v.min.map(TryInto::try_into).transpose()?,
                max: v.max.map(TryInto::try_into).transpose()?,
            },
            P::Object(v) => Self::Object {
                fields: decode(v.fields)?,
            },
            P::Contains(v) => Self::Contains {
                predicate: Box::new((*required(v.predicate)?).try_into()?),
                min_count: v.min_count,
                max_count: v.max_count,
            },
            P::AllOf(v) => Self::AllOf {
                predicates: v
                    .predicates
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_>>()?,
            },
        })
    }
}
impl TryFrom<Scope> for w::Scope {
    type Error = DataError;
    fn try_from(v: Scope) -> Result<Self> {
        Ok(Self {
            venue_id: v.venue_id,
            item_type_id: v.item_type_id,
            currency_id: v.currency_id,
            modes: v
                .modes
                .into_iter()
                .map(|v| w::OrderMode::from(v) as i32)
                .collect(),
            attributes: encode(v.attributes)?,
            price: v.price.map(|p| w::PriceRange {
                min: p.min,
                max: p.max,
            }),
        })
    }
}
impl TryFrom<w::Order> for Order {
    type Error = DataError;
    fn try_from(v: w::Order) -> Result<Self> {
        Ok(Self {
            id: v.id,
            book: required(v.book)?.into(),
            mode: match w::OrderMode::try_from(v.mode).map_err(|_| missing())? {
                w::OrderMode::Ask => Mode::Ask,
                w::OrderMode::Bid => Mode::Bid,
                w::OrderMode::Executed => Mode::Executed,
                _ => return Err(missing()),
            },
            item_id: v.item_id,
            attributes: decode(v.attributes)?,
            price: v.price,
            quantity: v.quantity,
            observed_at: time(v.observed_at_ms)?,
            expires_at: v.expires_at_ms.map(time).transpose()?,
            executed_at: v.executed_at_ms.map(time).transpose()?,
            parent_order_id: v.parent_order_id,
            meta: v.meta,
        })
    }
}
pub fn orders(result: market_replica::ReadResult) -> Result<Page<Order>> {
    let v = result.page;
    Ok(Page {
        sync: result.sync.into(),
        data: v
            .entities
            .into_iter()
            .map(|e| match required(e.entity)? {
                w::entity::Entity::Order(o) => o.try_into(),
                _ => Err(missing()),
            })
            .collect::<Result<_>>()?,
        next_page_token: v.next_page_token,
        cursor: v.cursor,
    })
}
fn aliases(v: Vec<w::ExternalId>) -> Vec<Alias> {
    v.into_iter()
        .map(|v| Alias {
            namespace: v.namespace,
            id: v.id,
        })
        .collect()
}
fn rules(v: BTreeMap<String, w::AttributeRule>) -> BTreeMap<String, AttributeRule> {
    v.into_iter()
        .map(|(k, v)| {
            (
                k,
                AttributeRule {
                    required: v.required,
                    allowed_values: v.allowed_values.map(|v| v.ids),
                },
            )
        })
        .collect()
}
pub fn catalog(result: market_replica::ReadResult) -> Result<Page<CatalogEntry>> {
    let v = result.page;
    Ok(Page {
        sync: result.sync.into(),
        data: v
            .entities
            .into_iter()
            .map(|e| {
                use w::entity::Entity as E;
                Ok(match required(e.entity)? {
                    E::Venue(v) => CatalogEntry::Venue {
                        id: v.id,
                        name: v.name,
                    },
                    E::Currency(v) => CatalogEntry::Currency {
                        id: v.id,
                        name: v.name,
                        decimals: v.decimals,
                    },
                    E::ItemType(v) => CatalogEntry::ItemType {
                        id: v.id,
                        name: v.name,
                        quantity_step: v.quantity_step,
                        aliases: aliases(v.aliases),
                        attributes: rules(v.attributes),
                    },
                    E::Attribute(v) => CatalogEntry::Attribute {
                        id: v.id,
                        name: v.name,
                        value_type: w::AttributeType::try_from(v.value_type)
                            .map_err(|_| missing())?
                            .as_str_name()
                            .trim_start_matches("ATTRIBUTE_TYPE_")
                            .to_lowercase(),
                        minimum: v.minimum.map(TryInto::try_into).transpose()?,
                        maximum: v.maximum.map(TryInto::try_into).transpose()?,
                        max_length: v.max_length,
                        element: v.element,
                        fields: rules(v.fields),
                    },
                    E::AttributeOption(v) => CatalogEntry::AttributeOption {
                        id: v.id,
                        name: v.name,
                        attribute_id: v.attribute_id,
                        aliases: aliases(v.aliases),
                        meta: v.meta,
                    },
                    E::Feed(v) => CatalogEntry::Feed {
                        id: v.id,
                        books: v.books.into_iter().map(Into::into).collect(),
                        modes: v
                            .modes
                            .into_iter()
                            .map(|mode| match w::OrderMode::try_from(mode) {
                                Ok(w::OrderMode::Ask) => Ok(Mode::Ask),
                                Ok(w::OrderMode::Bid) => Ok(Mode::Bid),
                                Ok(w::OrderMode::Executed) => Ok(Mode::Executed),
                                _ => Err(missing()),
                            })
                            .collect::<Result<_>>()?,
                        attributes: decode(v.attributes)?,
                        last_verified_at: v.last_verified_at_ms.map(time).transpose()?,
                    },
                    _ => return Err(missing()),
                })
            })
            .collect::<Result<_>>()?,
        next_page_token: v.next_page_token,
        cursor: v.cursor,
    })
}

impl From<CatalogKind> for w::EntityKind {
    fn from(value: CatalogKind) -> Self {
        match value {
            CatalogKind::Venue => w::EntityKind::Venue,
            CatalogKind::Currency => w::EntityKind::Currency,
            CatalogKind::ItemType => w::EntityKind::ItemType,
            CatalogKind::Attribute => w::EntityKind::Attribute,
            CatalogKind::AttributeOption => w::EntityKind::AttributeOption,
            CatalogKind::Feed => w::EntityKind::Feed,
        }
    }
}

impl From<market_replica::SyncStatus> for SyncStatus {
    fn from(value: market_replica::SyncStatus) -> Self {
        Self {
            ready: value.ready,
            state: match value.state {
                market_replica::SyncState::Starting => SyncState::Starting,
                market_replica::SyncState::Syncing => SyncState::Syncing,
                market_replica::SyncState::Live => SyncState::Live,
                market_replica::SyncState::Stale => SyncState::Stale,
            },
            updated_at: value.updated_at,
        }
    }
}
