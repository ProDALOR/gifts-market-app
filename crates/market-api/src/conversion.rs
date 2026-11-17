//! Map public API DTOs to actor-owned contracts. No hub or replica types.
use super::{
    error::{ApiError, Result},
    models::*,
};
use market_actors::models as m;
use std::collections::BTreeMap;

impl From<Mode> for m::Mode {
    fn from(v: Mode) -> Self {
        match v {
            Mode::Ask => Self::Ask,
            Mode::Bid => Self::Bid,
            Mode::Executed => Self::Executed,
        }
    }
}

impl From<Sort> for m::Sort {
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

impl From<CatalogKind> for m::CatalogKind {
    fn from(v: CatalogKind) -> Self {
        match v {
            CatalogKind::Venue => Self::Venue,
            CatalogKind::Currency => Self::Currency,
            CatalogKind::ItemType => Self::ItemType,
            CatalogKind::Attribute => Self::Attribute,
            CatalogKind::AttributeOption => Self::AttributeOption,
        }
    }
}

impl From<m::Mode> for Mode {
    fn from(v: m::Mode) -> Self {
        match v {
            m::Mode::Ask => Self::Ask,
            m::Mode::Bid => Self::Bid,
            m::Mode::Executed => Self::Executed,
        }
    }
}

impl From<BookKey> for m::BookKey {
    fn from(v: BookKey) -> Self {
        Self {
            venue_id: v.venue_id,
            item_type_id: v.item_type_id,
            currency_id: v.currency_id,
        }
    }
}

impl From<m::BookKey> for BookKey {
    fn from(v: m::BookKey) -> Self {
        Self {
            venue_id: v.venue_id,
            item_type_id: v.item_type_id,
            currency_id: v.currency_id,
        }
    }
}

impl From<AttributeValue> for m::AttributeValue {
    fn from(v: AttributeValue) -> Self {
        match v {
            AttributeValue::Enum { value } => Self::Enum { value },
            AttributeValue::Integer { value } => Self::Integer { value },
            AttributeValue::Decimal { value } => Self::Decimal { value },
            AttributeValue::Boolean { value } => Self::Boolean { value },
            AttributeValue::Text { value } => Self::Text { value },
            AttributeValue::Timestamp { value } => Self::Timestamp { value },
            AttributeValue::List { value } => Self::List {
                value: value.into_iter().map(Into::into).collect(),
            },
            AttributeValue::Object { value } => Self::Object { value: map(value) },
        }
    }
}

impl From<m::AttributeValue> for AttributeValue {
    fn from(v: m::AttributeValue) -> Self {
        match v {
            m::AttributeValue::Enum { value } => Self::Enum { value },
            m::AttributeValue::Integer { value } => Self::Integer { value },
            m::AttributeValue::Decimal { value } => Self::Decimal { value },
            m::AttributeValue::Boolean { value } => Self::Boolean { value },
            m::AttributeValue::Text { value } => Self::Text { value },
            m::AttributeValue::Timestamp { value } => Self::Timestamp { value },
            m::AttributeValue::List { value } => Self::List {
                value: value.into_iter().map(Into::into).collect(),
            },
            m::AttributeValue::Object { value } => Self::Object { value: map(value) },
        }
    }
}

impl From<Predicate> for m::Predicate {
    fn from(v: Predicate) -> Self {
        match v {
            Predicate::Eq { value } => Self::Eq {
                value: value.into(),
            },
            Predicate::OneOf { values } => Self::OneOf {
                values: values.into_iter().map(Into::into).collect(),
            },
            Predicate::Range { min, max } => Self::Range {
                min: min.map(Into::into),
                max: max.map(Into::into),
            },
            Predicate::Object { fields } => Self::Object {
                fields: map(fields),
            },
            Predicate::Contains {
                predicate,
                min_count,
                max_count,
            } => Self::Contains {
                predicate: Box::new((*predicate).into()),
                min_count,
                max_count,
            },
            Predicate::AllOf { predicates } => Self::AllOf {
                predicates: predicates.into_iter().map(Into::into).collect(),
            },
        }
    }
}

impl From<m::Predicate> for Predicate {
    fn from(v: m::Predicate) -> Self {
        match v {
            m::Predicate::Eq { value } => Self::Eq {
                value: value.into(),
            },
            m::Predicate::OneOf { values } => Self::OneOf {
                values: values.into_iter().map(Into::into).collect(),
            },
            m::Predicate::Range { min, max } => Self::Range {
                min: min.map(Into::into),
                max: max.map(Into::into),
            },
            m::Predicate::Object { fields } => Self::Object {
                fields: map(fields),
            },
            m::Predicate::Contains {
                predicate,
                min_count,
                max_count,
            } => Self::Contains {
                predicate: Box::new((*predicate).into()),
                min_count,
                max_count,
            },
            m::Predicate::AllOf { predicates } => Self::AllOf {
                predicates: predicates.into_iter().map(Into::into).collect(),
            },
        }
    }
}

fn map<T, U: From<T>>(values: BTreeMap<String, T>) -> BTreeMap<String, U> {
    values
        .into_iter()
        .map(|(key, value)| (key, value.into()))
        .collect()
}

impl From<PriceRange> for m::PriceRange {
    fn from(v: PriceRange) -> Self {
        Self {
            min: v.min,
            max: v.max,
        }
    }
}

impl From<Scope> for m::Scope {
    fn from(v: Scope) -> Self {
        Self {
            venue_id: v.venue_id,
            item_type_id: v.item_type_id,
            currency_id: v.currency_id,
            modes: v.modes.into_iter().map(Into::into).collect(),
            attributes: map(v.attributes),
            price: v.price.map(Into::into),
        }
    }
}

impl From<OrdersQuery> for m::OrdersQuery {
    fn from(v: OrdersQuery) -> Self {
        Self {
            scope: v.scope.into(),
            sort: v.sort.into(),
            consistency: match v.consistency {
                ReadConsistency::Snapshot => m::ReadConsistency::Snapshot,
                ReadConsistency::Live => m::ReadConsistency::Live,
            },
            limit: v.limit,
            page_token: v.page_token,
        }
    }
}

impl From<ExecutionsQuery> for m::ExecutionsQuery {
    fn from(v: ExecutionsQuery) -> Self {
        Self {
            scope: v.scope.into(),
            sort: v.sort.into(),
            period: v.period.map(Into::into),
            limit: v.limit,
            page_token: v.page_token,
        }
    }
}

impl From<Period> for m::Period {
    fn from(v: Period) -> Self {
        Self {
            from: v.from,
            until: v.until,
        }
    }
}

impl From<CatalogQuery> for m::CatalogQuery {
    fn from(v: CatalogQuery) -> Self {
        Self {
            kind: v.kind.into(),
            ids: v.ids,
            limit: v.limit,
            page_token: v.page_token,
        }
    }
}

impl From<BooksQuery> for m::BooksQuery {
    fn from(v: BooksQuery) -> Self {
        Self {
            after: v.after.map(Into::into),
            limit: v.limit,
        }
    }
}

impl From<m::Order> for Order {
    fn from(v: m::Order) -> Self {
        Self {
            id: v.id,
            book: v.book.into(),
            mode: v.mode.into(),
            item_id: v.item_id,
            attributes: map(v.attributes),
            price: v.price,
            quantity: v.quantity,
            observed_at: v.observed_at,
            expires_at: v.expires_at,
            executed_at: v.executed_at,
            parent_order_id: v.parent_order_id,
            meta: v.meta,
        }
    }
}

impl From<m::Alias> for Alias {
    fn from(v: m::Alias) -> Self {
        Self {
            namespace: v.namespace,
            id: v.id,
        }
    }
}

impl From<m::AttributeRule> for AttributeRule {
    fn from(v: m::AttributeRule) -> Self {
        Self {
            required: v.required,
            allowed_values: v.allowed_values,
        }
    }
}

impl TryFrom<m::CatalogEntry> for CatalogEntry {
    type Error = ApiError;
    fn try_from(v: m::CatalogEntry) -> Result<Self> {
        Ok(match v {
            m::CatalogEntry::Venue { id, name } => Self::Venue { id, name },
            m::CatalogEntry::Currency { id, name, decimals } => {
                Self::Currency { id, name, decimals }
            }
            m::CatalogEntry::ItemType {
                id,
                name,
                quantity_step,
                aliases,
                attributes,
            } => Self::ItemType {
                id,
                name,
                quantity_step,
                aliases: aliases.into_iter().map(Into::into).collect(),
                attributes: map(attributes),
            },
            m::CatalogEntry::Attribute {
                id,
                name,
                value_type,
                minimum,
                maximum,
                max_length,
                element,
                fields,
            } => Self::Attribute {
                id,
                name,
                value_type,
                minimum: minimum.map(Into::into),
                maximum: maximum.map(Into::into),
                max_length,
                element,
                fields: map(fields),
            },
            m::CatalogEntry::AttributeOption {
                id,
                name,
                attribute_id,
                aliases,
                meta,
            } => Self::AttributeOption {
                id,
                name,
                attribute_id,
                aliases: aliases.into_iter().map(Into::into).collect(),
                meta,
            },
            m::CatalogEntry::Feed { .. } => {
                return Err(market_actors::error::DataError::unavailable(
                    "unexpected catalog feed",
                )
                .into());
            }
        })
    }
}

pub fn orders(value: m::Page<m::Order>) -> Page<Order> {
    Page {
        data: value.data.into_iter().map(Into::into).collect(),
        next_page_token: value.next_page_token,
        cursor: value.cursor,
        sync: value.sync.into(),
    }
}
pub fn catalog(value: m::Page<m::CatalogEntry>) -> Result<Page<CatalogEntry>> {
    Ok(Page {
        data: value
            .data
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_>>()?,
        next_page_token: value.next_page_token,
        cursor: value.cursor,
        sync: value.sync.into(),
    })
}
impl From<market_actors::SyncStatus> for SyncInfo {
    fn from(value: market_actors::SyncStatus) -> Self {
        Self {
            ready: value.ready,
            state: match value.state {
                market_actors::SyncState::Starting => SyncState::Starting,
                market_actors::SyncState::Syncing => SyncState::Syncing,
                market_actors::SyncState::Live => SyncState::Live,
                market_actors::SyncState::Stale => SyncState::Stale,
            },
            updated_at: value
                .updated_at
                .and_then(|time| jiff::Timestamp::from_millisecond(time).ok()),
        }
    }
}
