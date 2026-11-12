use super::*;
use crate::pagination::{self, MAX_RESPONSE_BYTES};
use market_query::{BookCursor, BookFilter, BookId as StateBookId, BookQuery as StateBookQuery};
use market_view::{self as m, CatalogEntry, OrderSide, Record, RecordId, RecordKey};
use rust_decimal::Decimal;
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

fn common(
    catalog: &m::Catalog,
    feed: Option<&m::FeedId>,
    kind: Option<&m::ItemTypeId>,
) -> Result<()> {
    if feed.is_some_and(|id| catalog.feed(id).is_none()) {
        return Err(ApiError::field("feed_id", "unknown feed"));
    }
    if kind.is_some_and(|id| catalog.item_type(id).is_none()) {
        return Err(ApiError::field("item_type_id", "unknown item type"));
    }
    Ok(())
}
fn attributes(
    catalog: &m::Catalog,
    kind: Option<&m::ItemTypeId>,
    raw: Option<&str>,
) -> Result<m::Attributes> {
    let Some(raw) = raw else {
        return Ok(m::Attributes::new());
    };
    if raw.len() > 16 * 1024 {
        return Err(ApiError::field(
            "attributes",
            "candidate attributes are too large",
        ));
    }
    let kind = kind.ok_or_else(|| ApiError::field("item_type_id", "required for attributes"))?;
    market_json::parse_attributes(raw.as_bytes(), catalog, kind)
        .map_err(|error| ApiError::field("attributes", error))
}
fn normalized(raw: &mut Option<String>) -> Result<()> {
    if let Some(text) = raw {
        if text.len() > 16 * 1024 {
            return Err(ApiError::field(
                "attributes",
                "candidate attributes are too large",
            ));
        }
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| ApiError::field("attributes", error))?;
        *text = serde_json::to_string(&value).expect("JSON serializes");
    }
    Ok(())
}
fn size(value: &serde_json::Value) -> usize {
    serde_json::to_vec(value)
        .expect("domain value serializes")
        .len()
}
fn wrong_position() -> ApiError {
    ApiError::new(
        ErrorCode::InvalidCursor,
        "cursor position does not match endpoint",
    )
}

impl ApiState {
    pub fn catalog(&self, query: &CatalogQuery) -> Result<CatalogResponse> {
        let limit = pagination::limit(query.limit)?;
        let mut identity = query.clone();
        identity.limit = None;
        identity.cursor = None;
        if let Some(id) = &query.id {
            m::validate_id(id).map_err(|error| ApiError::field("id", error))?;
        }
        if query.q.as_ref().is_some_and(|q| q.len() > 256) {
            return Err(ApiError::field("q", "search is too long"));
        }
        if query.kind.as_ref().is_some_and(|kind| {
            ![
                "venue",
                "currency",
                "feed",
                "item_type",
                "attribute",
                "attribute_value",
            ]
            .contains(&kind.as_str())
        }) {
            return Err(ApiError::field("kind", "unknown catalog kind"));
        }
        let (context, _permit) = self.context(&identity, query.cursor.as_deref())?;
        let catalog = &context.view.catalog.catalog;
        common(catalog, None, query.item_type_id.as_ref())?;
        if query
            .attribute_id
            .as_ref()
            .is_some_and(|id| catalog.attribute(id).is_none())
        {
            return Err(ApiError::field("attribute_id", "unknown attribute"));
        }
        let start = match context.position {
            Some(Position::Catalog(offset)) => offset,
            None => 0,
            _ => return Err(wrong_position()),
        };
        if start > context.view.catalog.entries.len() {
            return Err(wrong_position());
        }
        let scope = query
            .item_type_id
            .as_ref()
            .and_then(|id| catalog.item_type(id));
        let search = query.q.as_ref().map(|q| q.to_lowercase());
        let matches = |entry: &CatalogEntry| {
            let (kind, id, name) = match entry {
                CatalogEntry::Venue(v) => ("venue", v.id.as_str(), v.name.as_str()),
                CatalogEntry::Currency(v) => ("currency", v.id.as_str(), v.name.as_str()),
                CatalogEntry::Feed(v) => ("feed", v.id.as_str(), v.id.as_str()),
                CatalogEntry::ItemType(v) => ("item_type", v.id.as_str(), v.name.as_str()),
                CatalogEntry::Attribute(v) => ("attribute", v.id.as_str(), v.name.as_str()),
                CatalogEntry::AttributeValue(v) => {
                    ("attribute_value", v.id.as_str(), v.name.as_str())
                }
            };
            if query.kind.as_deref().is_some_and(|wanted| wanted != kind)
                || query.id.as_deref().is_some_and(|wanted| wanted != id)
                || search.as_ref().is_some_and(|q| {
                    !name.to_lowercase().contains(q) && !id.to_lowercase().contains(q)
                })
            {
                return false;
            }
            if let Some(scope) = scope {
                match entry {
                    CatalogEntry::ItemType(v) if v.id == scope.id => {}
                    CatalogEntry::Attribute(v) if scope.attributes.contains_key(&v.id) => {}
                    CatalogEntry::AttributeValue(v)
                        if scope.attributes.get(&v.attribute_id).is_some_and(|rule| {
                            rule.allowed_values
                                .as_ref()
                                .is_none_or(|values| values.contains(&v.id))
                        }) => {}
                    _ => return false,
                }
            }
            if let Some(id) = &query.attribute_id {
                match entry {
                    CatalogEntry::Attribute(v) if &v.id == id => {}
                    CatalogEntry::AttributeValue(v) if &v.attribute_id == id => {}
                    _ => return false,
                }
            }
            true
        };
        let mut data = Vec::new();
        let mut bytes = 0;
        let mut next = None;
        for (index, entry) in context.view.catalog.entries.iter().enumerate().skip(start) {
            if !matches(entry) {
                continue;
            }
            let len = size(&market_json::catalog_value(entry).expect("validated catalog"));
            if len > MAX_RESPONSE_BYTES {
                return Err(ApiError::field(
                    "id",
                    "catalog entry exceeds response limit",
                ));
            }
            if data.len() == limit || bytes + len > MAX_RESPONSE_BYTES {
                next = Some(index);
                break;
            }
            bytes += len;
            data.push(entry.clone());
        }
        let next_cursor = next
            .map(|offset| self.cursor(&context, Position::Catalog(offset)))
            .transpose()?;
        Ok(Response {
            data: CatalogEntries(data),
            page: Some(Page { next_cursor }),
            meta: context.meta(),
            included: None,
        })
    }
    pub fn feeds(&self, query: &FeedsQuery) -> Result<FeedsResponse> {
        let (context, _permit) = self.context(query, None)?;
        common(context.view.state.catalog(), query.feed_id.as_ref(), None)?;
        let data = context
            .view
            .state
            .catalog()
            .feeds()
            .filter(|feed| query.feed_id.as_ref().is_none_or(|id| id == &feed.id))
            .filter_map(|feed| {
                match context.view.state.get(&RecordKey {
                    feed_id: feed.id.clone(),
                    id: RecordId::Feed(feed.id.clone()),
                }) {
                    Some(Record::Feed(status)) => Some(status.clone()),
                    _ => None,
                }
            })
            .collect();
        Ok(Response {
            data: Feeds(data),
            page: None,
            meta: context.meta(),
            included: None,
        })
    }
    pub fn items(&self, query: &ItemsQuery) -> Result<ItemsResponse> {
        let limit = pagination::limit(query.limit)?;
        let mut identity = query.clone();
        identity.limit = None;
        identity.cursor = None;
        normalized(&mut identity.attributes)?;
        let (context, _permit) = self.context(&identity, query.cursor.as_deref())?;
        common(
            context.view.state.catalog(),
            query.feed_id.as_ref(),
            query.item_type_id.as_ref(),
        )?;
        let traits = attributes(
            context.view.state.catalog(),
            query.item_type_id.as_ref(),
            query.attributes.as_deref(),
        )?;
        let after = match &context.position {
            Some(Position::Item(key)) => {
                key.validate().map_err(ApiError::invalid_query)?;
                Some(key)
            }
            None => None,
            _ => return Err(wrong_position()),
        };
        let mut iter = context.view.state.after(after).peekable();
        let mut data = Vec::new();
        let mut bytes = 0;
        let mut scanned = 0;
        let mut last = after.cloned();
        let mut overflow = false;
        while scanned < self.max_scan && data.len() < limit {
            let Some((key, record)) = iter.next() else {
                break;
            };
            if let Record::Item(item) = record
                && query.feed_id.as_ref().is_none_or(|id| id == &item.feed_id)
                && query
                    .item_type_id
                    .as_ref()
                    .is_none_or(|id| id == &item.item_type_id)
                && query.item_id.as_ref().is_none_or(|id| id == &item.item_id)
                && traits
                    .iter()
                    .all(|(id, value)| item.attributes.get(id) == Some(value))
            {
                let len = size(&market_json::record_value(record).expect("validated item"));
                if len > MAX_RESPONSE_BYTES {
                    return Err(ApiError::field("limit", "item exceeds response limit"));
                }
                if bytes + len > MAX_RESPONSE_BYTES {
                    overflow = true;
                    break;
                }
                bytes += len;
                data.push(item.clone());
            }
            last = Some(key.clone());
            scanned += 1;
        }
        let next_cursor = if overflow || iter.peek().is_some() {
            last.map(|key| self.cursor(&context, Position::Item(key)))
                .transpose()?
        } else {
            None
        };
        Ok(Response {
            data: Items(data),
            page: Some(Page { next_cursor }),
            meta: context.meta(),
            included: None,
        })
    }
    pub fn orders(&self, query: &OrdersQuery) -> Result<OrdersResponse> {
        let limit = pagination::limit(query.limit)?;
        let mut identity = query.clone();
        identity.limit = None;
        identity.cursor = None;
        normalized(&mut identity.attributes)?;
        let (context, _permit) = self.context(&identity, query.cursor.as_deref())?;
        let data = self.order_page(&context, query, limit)?;
        let included = included(&context.view, &data.data.0, query.include.as_deref())?;
        Ok(Response {
            data: data.data,
            page: Some(data.page),
            meta: context.meta(),
            included,
        })
    }
    pub fn orderbook(&self, query: &BookQuery) -> Result<BookResponse> {
        let limit = pagination::limit(query.limit)?;
        let asks_query = query.orders(OrderSide::Ask);
        let bids_query = query.orders(OrderSide::Bid);
        let mut identity = asks_query.clone();
        identity.limit = None;
        normalized(&mut identity.attributes)?;
        let (mut context, _permit) = self.context(&identity, None)?;
        let asks = self.order_page(&context, &asks_query, limit)?;
        identity.side = Some(OrderSide::Bid);
        context.hash = fingerprint(&identity);
        let bids = self.order_page(&context, &bids_query, limit)?;
        let all: Vec<_> = asks
            .data
            .0
            .iter()
            .chain(bids.data.0.iter())
            .cloned()
            .collect();
        let included = included(&context.view, &all, query.include.as_deref())?;
        Ok(Response {
            data: BookData {
                book: models::BookId {
                    item_type_id: query.item_type_id.clone(),
                    currency_id: query.currency_id.clone(),
                },
                asks,
                bids,
            },
            page: None,
            meta: context.meta(),
            included,
        })
    }
    fn order_page(
        &self,
        context: &QueryView,
        query: &OrdersQuery,
        limit: usize,
    ) -> Result<OrderPage> {
        common(
            context.view.state.catalog(),
            query.feed_id.as_ref(),
            query.item_type_id.as_ref(),
        )?;
        let minimum = bound(query.price_min.as_deref(), "price_min")?;
        let maximum = bound(query.price_max.as_deref(), "price_max")?;
        if minimum.zip(maximum).is_some_and(|(min, max)| min > max) {
            return Err(ApiError::field("price_min", "exceeds price_max"));
        }
        let price_matches = |order: &m::Order| {
            minimum.is_none_or(|min| order.amounts.price.amount >= min)
                && maximum.is_none_or(|max| order.amounts.price.amount <= max)
        };
        if let Some(id) = &query.order_id {
            if context.position.is_some() {
                return Err(wrong_position());
            }
            let feed = query
                .feed_id
                .as_ref()
                .ok_or_else(|| ApiError::field("feed_id", "required for order lookup"))?;
            if query.attributes.is_some() || query.item_id.is_some() {
                return Err(ApiError::field(
                    "order_id",
                    "lookup does not accept candidate attributes or item filters",
                ));
            }
            let mut data = Vec::new();
            for (side, key) in [
                (OrderSide::Ask, RecordId::Ask(id.clone())),
                (OrderSide::Bid, RecordId::Bid(id.clone())),
            ] {
                if query.side.is_some_and(|wanted| wanted != side) {
                    continue;
                }
                if let Some(Record::Order(order)) = context.view.state.get(&RecordKey {
                    feed_id: feed.clone(),
                    id: key,
                }) && query
                    .item_type_id
                    .as_ref()
                    .is_none_or(|id| id == &order.item_type_id)
                    && query
                        .currency_id
                        .as_ref()
                        .is_none_or(|id| id == &order.amounts.price.currency_id)
                    && price_matches(order)
                {
                    data.push(order.clone());
                }
            }
            return Ok(OrderPage {
                data: Orders(data),
                page: Page { next_cursor: None },
            });
        }
        let item_type_id = query
            .item_type_id
            .as_ref()
            .ok_or_else(|| ApiError::field("item_type_id", "required for market queries"))?;
        let currency_id = query
            .currency_id
            .as_ref()
            .ok_or_else(|| ApiError::field("currency_id", "required for market queries"))?;
        let side = query
            .side
            .ok_or_else(|| ApiError::field("side", "required for market queries"))?;
        let book = StateBookId {
            item_type_id: item_type_id.clone(),
            currency_id: currency_id.clone(),
        };
        let filter = BookFilter {
            feed_id: query.feed_id.clone(),
            item_id: query.item_id.clone(),
            attributes: attributes(
                context.view.state.catalog(),
                Some(item_type_id),
                query.attributes.as_deref(),
            )?,
        };
        let after = match &context.position {
            Some(Position::Order(cursor)) => Some(cursor),
            None => None,
            _ => return Err(wrong_position()),
        };
        let mut position = after.cloned();
        let mut scanned = 0;
        let mut data = Vec::new();
        let mut bytes = 0;
        let mut next = None;
        'pages: while data.len() < limit && scanned < self.max_scan {
            let page = context
                .view
                .state
                .book_page(StateBookQuery {
                    book: &book,
                    side,
                    filter: &filter,
                    after: position.as_ref(),
                    now: context.as_of,
                    limit: limit - data.len(),
                    max_scan: self.max_scan - scanned,
                })
                .map_err(ApiError::invalid_query)?;
            scanned += page.scanned;
            for order in page.orders {
                if (side == OrderSide::Ask
                    && maximum.is_some_and(|max| order.amounts.price.amount > max))
                    || (side == OrderSide::Bid
                        && minimum.is_some_and(|min| order.amounts.price.amount < min))
                {
                    next = None;
                    break 'pages;
                }
                let cursor = BookCursor::for_order(&order);
                if !price_matches(&order) {
                    position = Some(cursor);
                    continue;
                }
                let len = size(&market_json::order_value(&order).expect("validated order"));
                if len > MAX_RESPONSE_BYTES {
                    return Err(ApiError::field("limit", "order exceeds response limit"));
                }
                if bytes + len > MAX_RESPONSE_BYTES {
                    next = position;
                    break 'pages;
                }
                bytes += len;
                position = Some(cursor);
                data.push(order);
            }
            next = page.next_after;
            if next.is_none() {
                break;
            }
            position = next.clone();
        }
        let next_cursor = next
            .map(|position| self.cursor(context, Position::Order(position)))
            .transpose()?;
        Ok(OrderPage {
            data: Orders(data),
            page: Page { next_cursor },
        })
    }
}
fn bound(raw: Option<&str>, field: &str) -> Result<Option<Decimal>> {
    raw.map(|raw| {
        if raw.len() > 64 {
            return Err(ApiError::field(field, "decimal is too long"));
        }
        let value = Decimal::from_str(raw).map_err(|error| ApiError::field(field, error))?;
        if value < Decimal::ZERO {
            return Err(ApiError::field(field, "must not be negative"));
        }
        Ok(value)
    })
    .transpose()
}
fn included(
    view: &ReplicaView,
    orders: &[m::Order],
    raw: Option<&str>,
) -> Result<Option<Included>> {
    let Some(raw) = raw else { return Ok(None) };
    let mut want_item = false;
    let mut want_feed = false;
    for value in raw.split(',') {
        match value {
            "item" => want_item = true,
            "feed" => want_feed = true,
            _ => {
                return Err(ApiError::field(
                    "include",
                    "supported dependencies are item,feed",
                ));
            }
        }
    }
    let mut items = BTreeMap::new();
    let mut feeds = BTreeSet::new();
    for order in orders {
        feeds.insert(order.feed_id.clone());
        if want_item
            && let Some(item) = order
                .item_id()
                .and_then(|id| view.state.item_observation(id))
        {
            feeds.insert(item.feed_id.clone());
            items.insert(item.key(), item.clone());
        }
    }
    let items: Vec<_> = items.into_values().collect();
    let bytes: usize = items
        .iter()
        .map(|item| {
            size(&market_json::record_value(&Record::Item(item.clone())).expect("validated item"))
        })
        .sum();
    if bytes > MAX_RESPONSE_BYTES {
        return Err(ApiError::field(
            "limit",
            "included items exceed response limit; lower limit",
        ));
    }
    let feeds = if want_feed {
        feeds
            .into_iter()
            .filter_map(|id| {
                match view.state.get(&RecordKey {
                    feed_id: id.clone(),
                    id: RecordId::Feed(id),
                }) {
                    Some(Record::Feed(feed)) => Some(feed.clone()),
                    _ => None,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Some(Included { items, feeds }))
}
