//! Derived indexes have one insertion/removal path; canonical records live in MarketState.
use crate::book::{BookId, BookIndex};
use im::OrdMap;
use market_view::{
    FeedId, FeedState, ItemId, ItemTypeId, MarketTimestamp, Record, ValidationError,
};
use std::sync::Arc;
#[derive(Debug, Clone)]
pub(crate) struct ItemIndex {
    pub(crate) item_type_id: ItemTypeId,
    references: usize,
    pub(crate) observations: OrdMap<FeedId, MarketTimestamp>,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct Indexes {
    pub(crate) books: OrdMap<BookId, BookIndex>,
    pub(crate) items: OrdMap<ItemId, Arc<ItemIndex>>,
    pub(crate) feeds: OrdMap<FeedId, FeedState>,
}
impl Indexes {
    /// Compare only fields represented in derived indexes. Queries read remaining
    /// facts (attributes, filters, quantity, expiry and metadata) from canonical records.
    pub(crate) fn unchanged(old: &Record, new: &Record) -> bool {
        match (old, new) {
            (Record::Order(old), Record::Order(new)) => {
                old.feed_id == new.feed_id
                    && old.order_id == new.order_id
                    && old.side() == new.side()
                    && old.item_type_id == new.item_type_id
                    && old.amounts.price == new.amounts.price
                    && old.item_id() == new.item_id()
            }
            (Record::Item(old), Record::Item(new)) => {
                old.feed_id == new.feed_id
                    && old.item_id == new.item_id
                    && old.item_type_id == new.item_type_id
                    && old.observed_at == new.observed_at
            }
            (Record::Feed(old), Record::Feed(new)) => {
                old.feed_id == new.feed_id && old.state == new.state
            }
            _ => false,
        }
    }
    pub(crate) fn insert(&mut self, record: &Record) -> Result<(), ValidationError> {
        if let Some((id, kind)) = record.item_reference() {
            let mut item = match self.items.get(id) {
                Some(item) if &item.item_type_id != kind => {
                    return Err(ValidationError("item ID has conflicting item types"));
                }
                Some(item) => item.as_ref().clone(),
                None => ItemIndex {
                    item_type_id: kind.clone(),
                    references: 0,
                    observations: OrdMap::new(),
                },
            };
            item.references += 1;
            if let Record::Item(observation) = record {
                item.observations
                    .insert(observation.feed_id.clone(), observation.observed_at);
            }
            self.items.insert(id.clone(), Arc::new(item));
        }
        match record {
            Record::Order(order) => {
                let id = BookId::for_order(order);
                let mut book = self.books.get(&id).cloned().unwrap_or_default();
                book.insert(order);
                self.books.insert(id, book);
            }
            Record::Feed(feed) => {
                self.feeds.insert(feed.feed_id.clone(), feed.state);
            }
            _ => {}
        }
        Ok(())
    }
    pub(crate) fn remove(&mut self, record: &Record) {
        if let Some((id, _)) = record.item_reference() {
            let mut item = self
                .items
                .get(id)
                .expect("item reference indexed")
                .as_ref()
                .clone();
            item.references -= 1;
            if let Record::Item(observation) = record {
                item.observations.remove(&observation.feed_id);
            }
            if item.references == 0 {
                self.items.remove(id);
            } else {
                self.items.insert(id.clone(), Arc::new(item));
            }
        }
        match record {
            Record::Order(order) => {
                let id = BookId::for_order(order);
                let mut book = self.books.get(&id).expect("order indexed").clone();
                book.remove(order);
                if book.is_empty() {
                    self.books.remove(&id);
                } else {
                    self.books.insert(id, book);
                }
            }
            Record::Feed(feed) => {
                self.feeds.remove(&feed.feed_id);
            }
            _ => {}
        }
    }
}
