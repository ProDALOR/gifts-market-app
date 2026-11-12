//! Derived price indexes. Time and feed availability are evaluated against one immutable view.
use crate::MarketState;
use im::OrdMap;
use market_view::{
    Attributes, CurrencyId, FeedId, FeedState, Item, ItemId, ItemTypeId, MarketTimestamp, Order,
    OrderKind, Record, RecordId, RecordKey, ValidationError,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::ops::Bound::{Excluded, Unbounded};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookId {
    pub item_type_id: ItemTypeId,
    pub currency_id: CurrencyId,
}
pub use market_view::OrderSide as BookSide;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookFilter {
    pub feed_id: Option<FeedId>,
    pub item_id: Option<ItemId>,
    #[serde(
        default,
        deserialize_with = "market_view::attributes::deserialize_attribute_map"
    )]
    pub attributes: Attributes,
}
/// A position in a single side of a single book, including a deterministic tie-breaker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookCursor {
    pub book: BookId,
    pub side: BookSide,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    pub key: RecordKey,
}
pub struct BookQuery<'a> {
    pub book: &'a BookId,
    pub side: BookSide,
    pub filter: &'a BookFilter,
    pub after: Option<&'a BookCursor>,
    pub now: MarketTimestamp,
    pub limit: usize,
    pub max_scan: usize,
}
#[derive(Debug)]
pub struct BookPage {
    pub orders: Vec<Order>,
    pub next_after: Option<BookCursor>,
    pub scanned: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PriceKey {
    // Negative bid prices put highest bids first; order keys break ties in ascending order.
    sort_price: Decimal,
    key: RecordKey,
}
impl PriceKey {
    fn new(side: BookSide, price: Decimal, key: RecordKey) -> Self {
        Self {
            sort_price: match side {
                BookSide::Ask => price,
                BookSide::Bid => -price,
            },
            key,
        }
    }
    fn cursor(&self, book: &BookId, side: BookSide) -> BookCursor {
        BookCursor {
            book: book.clone(),
            side,
            price: match side {
                BookSide::Ask => self.sort_price,
                BookSide::Bid => -self.sort_price,
            },
            key: self.key.clone(),
        }
    }
}
#[derive(Debug, Clone, Default)]
pub(crate) struct BookIndex {
    asks: OrdMap<PriceKey, ()>,
    bids: OrdMap<PriceKey, ()>,
}
impl BookIndex {
    fn side(&self, side: BookSide) -> &OrdMap<PriceKey, ()> {
        match side {
            BookSide::Ask => &self.asks,
            BookSide::Bid => &self.bids,
        }
    }
    fn side_mut(&mut self, side: BookSide) -> &mut OrdMap<PriceKey, ()> {
        match side {
            BookSide::Ask => &mut self.asks,
            BookSide::Bid => &mut self.bids,
        }
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.asks.is_empty() && self.bids.is_empty()
    }
}
impl BookCursor {
    pub fn for_order(order: &Order) -> Self {
        let (book, side, key) = order_location(order);
        key.cursor(&book, side)
    }
}
fn order_location(order: &Order) -> (BookId, BookSide, PriceKey) {
    (
        BookId::for_order(order),
        order.side(),
        PriceKey::new(order.side(), order.amounts.price.amount, order.key()),
    )
}
impl BookId {
    pub(crate) fn for_order(order: &Order) -> Self {
        Self {
            item_type_id: order.item_type_id.clone(),
            currency_id: order.amounts.price.currency_id.clone(),
        }
    }
}
impl BookIndex {
    pub(crate) fn insert(&mut self, order: &Order) {
        let (_, side, key) = order_location(order);
        self.side_mut(side).insert(key, ());
    }
    pub(crate) fn remove(&mut self, order: &Order) {
        let (_, side, key) = order_location(order);
        self.side_mut(side).remove(&key);
    }
}
impl MarketState {
    pub fn feed_ready(&self, id: &FeedId) -> bool {
        self.indexes.feeds.get(id) == Some(&FeedState::Ready)
    }
    /// Select one observation; never combine traits from different feeds. Missing traits are unknown.
    pub fn item_observation(&self, id: &ItemId) -> Option<&Item> {
        let (feed, _) = self
            .indexes
            .items
            .get(id)?
            .observations
            .iter()
            .filter(|(feed, _)| self.feed_ready(feed))
            .max_by(|(a_feed, a_time), (b_feed, b_time)| (a_time, a_feed).cmp(&(b_time, b_feed)))?;
        match self.get(&RecordKey {
            feed_id: feed.clone(),
            id: RecordId::Item(id.clone()),
        }) {
            Some(Record::Item(item)) => Some(item),
            _ => unreachable!("indexed item exists"),
        }
    }
    pub fn book_ids(&self) -> impl Iterator<Item = &BookId> {
        self.indexes.books.keys()
    }
    pub fn book_page(&self, query: BookQuery<'_>) -> Result<BookPage, ValidationError> {
        let BookQuery {
            book,
            side,
            filter,
            after,
            now,
            limit,
            max_scan,
        } = query;
        if limit == 0 || limit > 200 || max_scan == 0 {
            return Err(ValidationError("invalid book query limits"));
        }
        if self.catalog().item_type(&book.item_type_id).is_none()
            || self.catalog().currency(&book.currency_id).is_none()
        {
            return Err(ValidationError("unknown book item type or currency"));
        }
        if filter
            .feed_id
            .as_ref()
            .is_some_and(|id| self.catalog().feed(id).is_none())
        {
            return Err(ValidationError("unknown book feed"));
        }
        self.catalog()
            .validate_attributes(&book.item_type_id, &filter.attributes)?;
        if let Some(cursor) = after {
            if cursor.book != *book
                || cursor.side != side
                || cursor.price <= Decimal::ZERO
                || cursor.key.kind()
                    != match side {
                        BookSide::Ask => market_view::RecordKind::Ask,
                        BookSide::Bid => market_view::RecordKind::Bid,
                    }
            {
                return Err(ValidationError("book cursor does not match query"));
            }
            cursor.key.validate()?;
        }
        if let Some(id) = &filter.item_id
            && self
                .indexes
                .items
                .get(id)
                .is_some_and(|item| item.item_type_id != book.item_type_id)
        {
            return Err(ValidationError("item does not belong to requested book"));
        }
        let Some(index) = self.indexes.books.get(book) else {
            return Ok(BookPage {
                orders: Vec::new(),
                next_after: None,
                scanned: 0,
            });
        };
        let start = after.map(|c| PriceKey::new(side, c.price, c.key.clone()));
        let mut iter = match start {
            Some(key) => index.side(side).range((Excluded(key), Unbounded)),
            None => index.side(side).iter(),
        }
        .peekable();
        let item = filter
            .item_id
            .as_ref()
            .and_then(|id| self.item_observation(id));
        let mut orders = Vec::new();
        let mut scanned = 0;
        let mut last = None;
        while scanned < max_scan && orders.len() < limit {
            let Some((position, _)) = iter.next() else {
                break;
            };
            scanned += 1;
            last = Some(position);
            let order = self.get(&position.key).expect("indexed order exists");
            if filter
                .feed_id
                .as_ref()
                .is_some_and(|id| id != &position.key.feed_id)
                || !self.feed_ready(&position.key.feed_id)
            {
                continue;
            }
            let Record::Order(order) = order else {
                unreachable!("only orders indexed");
            };
            if order.expires_at.is_some_and(|expiry| expiry <= now) {
                continue;
            }
            let matches = match &order.kind {
                OrderKind::AskItem { item_id } => {
                    filter.item_id.as_ref().is_none_or(|id| id == item_id)
                        && (filter.attributes.is_empty()
                            || self.item_observation(item_id).is_some_and(|item| {
                                traits_match(&filter.attributes, &item.attributes)
                            }))
                }
                OrderKind::AskUnits { attributes } => {
                    filter.item_id.is_none() && traits_match(&filter.attributes, attributes)
                }
                _ => Self::bid_matches(order, filter, item),
            };
            if matches {
                orders.push(order.clone());
            }
        }
        let next_after = if iter.peek().is_some() {
            last.map(|key| key.cursor(book, side))
        } else {
            None
        };
        Ok(BookPage {
            orders,
            next_after,
            scanned,
        })
    }
    fn bid_matches(order: &Order, filter: &BookFilter, item: Option<&Item>) -> bool {
        if let Some(id) = &filter.item_id {
            if let Some(item) = item {
                return traits_match(&filter.attributes, &item.attributes)
                    && order.matches_item(item);
            }
            if !filter.attributes.is_empty() {
                return false;
            }
            return match &order.kind {
                OrderKind::BidItem { item_id, .. } => item_id == id,
                OrderKind::BidAny => true,
                _ => false,
            };
        }
        if filter.attributes.is_empty() {
            return true;
        }
        match &order.kind {
            OrderKind::BidAny => true,
            OrderKind::BidAttributes { filter: accepted } => {
                market_view::attribute_filter_matches(accepted, &filter.attributes)
            }
            _ => false,
        }
    }
}
fn traits_match(required: &Attributes, actual: &Attributes) -> bool {
    required
        .iter()
        .all(|(kind, id)| actual.get(kind) == Some(id))
}
