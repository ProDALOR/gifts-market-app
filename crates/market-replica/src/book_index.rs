//! Partition-owned active orders, covering indexes and a shared book directory.
use crate::{
    query::Scope,
    state::{Key, order},
    types::{ActiveMode, RankKey, Sort},
};
use im::{OrdMap, OrdSet};
use market_hub_protocol::wire as w;
use rust_decimal::Decimal;
use std::{cmp::Reverse, collections::BinaryHeap, ops::Bound, sync::Arc};
use tonic::Status;
mod attributes;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Book {
    venue_id: String,
    item_type_id: String,
    currency_id: String,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BookGroup {
    item_type_id: String,
    currency_id: String,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BookPartition {
    book: Book,
    mode: ActiveMode,
}
impl Book {
    fn group(&self) -> BookGroup {
        BookGroup {
            item_type_id: self.item_type_id.clone(),
            currency_id: self.currency_id.clone(),
        }
    }
}
impl From<&w::BookKey> for Book {
    fn from(value: &w::BookKey) -> Self {
        Self {
            venue_id: value.venue_id.clone(),
            item_type_id: value.item_type_id.clone(),
            currency_id: value.currency_id.clone(),
        }
    }
}
impl From<Book> for w::BookKey {
    fn from(value: Book) -> Self {
        Self {
            venue_id: value.venue_id,
            item_type_id: value.item_type_id,
            currency_id: value.currency_id,
        }
    }
}

/// Rank keys cover the authoritative record without duplicating its payload.
type RankedOrders = OrdMap<RankKey, Arc<w::Entity>>;
type RankedOrder<'a> = (&'a RankKey, &'a Arc<w::Entity>);
type RankedIterator<'a> = Box<dyn Iterator<Item = RankedOrder<'a>> + 'a>;

/// The three indexes of one venue/collection/currency/side share immutable payloads.
#[derive(Clone, Default)]
struct PartitionState {
    attributes: attributes::AttributeIndex,
    orders: OrdMap<String, Arc<w::Entity>>,
    prices: RankedOrders,
    times: RankedOrders,
    numbers: RankedOrders,
}
impl PartitionState {
    fn index(&self, sort: Sort) -> &RankedOrders {
        match sort {
            Sort::TimeAsc | Sort::TimeDesc => &self.times,
            Sort::NumberAsc | Sort::NumberDesc => &self.numbers,
            _ => &self.prices,
        }
    }
    fn indexes_mut(&mut self) -> [&mut RankedOrders; 3] {
        [&mut self.prices, &mut self.times, &mut self.numbers]
    }
    fn replace(
        &mut self,
        key: &Arc<Key>,
        previous: [Decimal; 3],
        next: [Decimal; 3],
        entity: &Arc<w::Entity>,
    ) {
        self.attributes.replace(
            key,
            self.orders.get(&key.id).expect("located order"),
            entity,
        );
        for (i, (index, rank)) in self.indexes_mut().into_iter().zip(next).enumerate() {
            if previous[i] != rank {
                index.remove(&RankKey::shared(previous[i], Arc::clone(key)));
            }
        }
        self.insert_records(key, next, entity);
    }
    fn remove(&mut self, key: &Arc<Key>, ranks: [Decimal; 3]) {
        if let Some(entity) = self.orders.get(&key.id) {
            self.attributes.remove(key, entity);
        }
        for (index, rank) in self.indexes_mut().into_iter().zip(ranks) {
            index.remove(&RankKey::shared(rank, Arc::clone(key)));
        }
        self.orders.remove(&key.id);
    }
    fn insert(&mut self, key: &Arc<Key>, ranks: [Decimal; 3], entity: &Arc<w::Entity>) {
        self.attributes.insert(key, entity);
        self.insert_records(key, ranks, entity);
    }
    fn insert_records(&mut self, key: &Arc<Key>, ranks: [Decimal; 3], entity: &Arc<w::Entity>) {
        // Equal ranks still need the new record, including metadata-only updates.
        for (index, rank) in self.indexes_mut().into_iter().zip(ranks) {
            index.insert(RankKey::shared(rank, Arc::clone(key)), Arc::clone(entity));
        }
        self.orders.insert(key.id.clone(), Arc::clone(entity));
    }
    fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }
}

/// Validate all placement/rank data before mutating any index.
struct OrderPlacement {
    partition: BookPartition,
    ranks: [Decimal; 3],
}
impl OrderPlacement {
    fn new(order: &w::Order) -> Result<Self, Status> {
        Ok(Self {
            partition: BookPartition {
                book: Book::from(
                    order
                        .book
                        .as_ref()
                        .ok_or_else(|| Status::data_loss("missing book"))?,
                ),
                mode: ActiveMode::try_from(order.mode)?,
            },
            ranks: ranks(order)?,
        })
    }
}

fn ranks(order: &w::Order) -> Result<[Decimal; 3], Status> {
    Ok([
        Sort::PriceAsc.rank(order, false)?,
        Sort::TimeAsc.rank(order, false)?,
        Sort::NumberAsc.rank(order, false)?,
    ])
}

// Number ranking and integer predicates use the same exact values.
fn number_bounds(predicate: Option<&w::AttributePredicate>) -> (Option<Decimal>, Option<Decimal>) {
    let integer = |value: &w::AttributeValue| match value.value.as_ref() {
        Some(w::attribute_value::Value::Integer(n)) => Some(Decimal::from(*n)),
        _ => None,
    };
    match predicate.and_then(|p| p.predicate.as_ref()) {
        Some(w::attribute_predicate::Predicate::Eq(value)) => {
            let value = integer(value);
            (value, value)
        }
        Some(w::attribute_predicate::Predicate::Range(range)) => (
            range.min.as_ref().and_then(integer),
            range.max.as_ref().and_then(integer),
        ),
        _ => (None, None),
    }
}

// Only rank and source position participate in heap ordering; records stay borrowed.
fn merge<'a, K: Ord + 'a>(
    mut iterators: Vec<RankedIterator<'a>>,
    priority: impl Fn(&'a RankKey) -> K + 'a,
) -> RankedIterator<'a> {
    let mut heads: Vec<_> = iterators.iter_mut().map(Iterator::next).collect();
    let mut heap = BinaryHeap::new();
    for (i, head) in heads.iter().enumerate() {
        if let Some((rank, _)) = head {
            heap.push((priority(rank), i));
        }
    }
    Box::new(std::iter::from_fn(move || {
        let (_, i) = heap.pop()?;
        let row = heads[i].take()?;
        heads[i] = iterators[i].next();
        if let Some((rank, _)) = heads[i] {
            heap.push((priority(rank), i));
        }
        Some(row)
    }))
}

#[derive(Clone, Default)]
pub struct BookIndex {
    // Copy-on-write at the partition boundary keeps unrelated index roots shared.
    partitions: OrdMap<Arc<BookPartition>, Arc<PartitionState>>,
    // Location keys preserve the legacy global identity order. Partition keys are interned.
    locations: OrdMap<Arc<Key>, Arc<BookPartition>>,
    groups: OrdMap<BookGroup, OrdSet<Book>>,
    declared: OrdMap<Key, OrdSet<Book>>,
    references: OrdMap<Book, usize>,
}
/// A scoped immutable active-order view. Only the selected partitions are retained;
/// history, metadata and unrelated books are not pinned. Callers bound traversal work.
#[derive(Clone)]
pub struct OrderBookSnapshot {
    index: BookIndex,
    scope: Scope,
}
impl OrderBookSnapshot {
    pub(crate) fn new(index: &BookIndex, scope: Scope) -> Self {
        let group = BookGroup {
            item_type_id: scope.item_type_id.clone().expect("scoped collection"),
            currency_id: scope.currency_id.clone().expect("scoped currency"),
        };
        let mut selected = BookIndex::default();
        for book in index.groups.get(&group).into_iter().flatten() {
            for mode in [ActiveMode::Ask, ActiveMode::Bid] {
                if !scope.modes.contains(&mode.into()) {
                    continue;
                }
                let key = BookPartition {
                    book: book.clone(),
                    mode,
                };
                if let Some((key, partition)) = index.partitions.get_key_value(&key) {
                    selected.partitions.insert(key.clone(), partition.clone());
                    selected
                        .groups
                        .entry(group.clone())
                        .or_default()
                        .insert(book.clone());
                }
            }
        }
        Self {
            index: selected,
            scope,
        }
    }
    /// Price-ordered candidates, narrowed by available attribute indexes. The caller
    /// checks exact predicates, eligibility, cancellation and its own scan budget.
    pub fn candidates(
        &self,
        attributes: std::collections::BTreeMap<String, w::AttributePredicate>,
    ) -> impl Iterator<Item = &Arc<w::Entity>> {
        let mut scope = self.scope.clone();
        scope.attributes = attributes;
        self.index
            .ranked(&scope, None, Sort::PriceAsc)
            .map(|(_, entity)| entity)
    }
}
impl BookIndex {
    pub(crate) fn contains(&self, key: &w::BookKey) -> bool {
        self.references.contains_key(&Book::from(key))
    }
    pub(crate) fn has_orders(&self, key: &w::BookKey) -> bool {
        self.active(&Book::from(key))
    }
    pub(crate) fn has_group(&self, item: &str, currency: &str) -> bool {
        self.groups.contains_key(&BookGroup {
            item_type_id: item.into(),
            currency_id: currency.into(),
        })
    }

    fn add_book(&mut self, book: Book) {
        *self.references.entry(book).or_default() += 1;
    }
    fn drop_book(&mut self, book: &Book) {
        if let Some(count) = self.references.get_mut(book) {
            *count -= 1;
            if *count == 0 {
                self.references.remove(book);
            }
        }
    }
    fn active(&self, book: &Book) -> bool {
        [ActiveMode::Ask, ActiveMode::Bid].into_iter().any(|mode| {
            self.partitions.contains_key(&BookPartition {
                book: book.clone(),
                mode,
            })
        })
    }
    fn record(&self, partition: &BookPartition, key: &Key) -> Option<&Arc<w::Entity>> {
        self.partitions.get(partition)?.orders.get(&key.id)
    }
    pub fn get(&self, key: &Key) -> Option<&Arc<w::Entity>> {
        self.record(self.locations.get(key)?, key)
    }
    pub fn range(
        &self,
        bounds: (Bound<Key>, Bound<Key>),
    ) -> impl DoubleEndedIterator<Item = (&Key, &Arc<w::Entity>)> {
        self.locations
            .range::<_, Key>(bounds)
            .map(|(key, partition)| {
                (
                    key.as_ref(),
                    self.record(partition, key)
                        .expect("order location must reference a stored order"),
                )
            })
    }
    pub fn upsert_order(&mut self, key: Key, entity: &Arc<w::Entity>) -> Result<(), Status> {
        let value = order(entity).ok_or_else(|| Status::data_loss("missing order"))?;
        let next = OrderPlacement::new(value)?;
        let previous = self
            .locations
            .get_key_value(&key)
            .map(|(key, location)| (Arc::clone(key), Arc::clone(location)));
        if let Some((identity, location)) = previous {
            let stored = self
                .record(&location, &identity)
                .ok_or_else(|| Status::data_loss("missing located order"))?;
            let previous_ranks =
                ranks(order(stored).ok_or_else(|| Status::data_loss("missing order"))?)?;
            if location.as_ref() == &next.partition {
                let partition = self
                    .partitions
                    .get_mut(&next.partition)
                    .ok_or_else(|| Status::data_loss("missing order partition"))?;
                Arc::make_mut(partition).replace(&identity, previous_ranks, next.ranks, entity);
            } else {
                self.remove_order(&identity, &location, previous_ranks);
                self.insert_placed(identity, entity, next);
            }
        } else {
            self.insert_placed(Arc::new(key), entity, next);
        }
        Ok(())
    }
    /// Only used for unique, identity-checked rows restored into an empty State.
    pub(crate) fn insert_restored(
        &mut self,
        key: Key,
        entity: &Arc<w::Entity>,
    ) -> Result<(), Status> {
        let next =
            OrderPlacement::new(order(entity).ok_or_else(|| Status::data_loss("missing order"))?)?;
        self.insert_placed(Arc::new(key), entity, next);
        Ok(())
    }
    fn insert_placed(&mut self, key: Arc<Key>, entity: &Arc<w::Entity>, next: OrderPlacement) {
        let location = if let Some((location, _)) = self.partitions.get_key_value(&next.partition) {
            Arc::clone(location)
        } else {
            let book = &next.partition.book;
            if !self.active(book) {
                self.add_book(book.clone());
                self.groups
                    .entry(book.group())
                    .or_default()
                    .insert(book.clone());
            }
            Arc::new(next.partition)
        };
        let partition = self.partitions.entry(Arc::clone(&location)).or_default();
        Arc::make_mut(partition).insert(&key, next.ranks, entity);
        self.locations.insert(key, location);
    }
    pub fn upsert_feed(&mut self, key: &Key, feed: &w::Feed) {
        self.remove_feed(key);
        let books: OrdSet<Book> = feed.books.iter().map(Book::from).collect();
        for book in &books {
            self.add_book(book.clone());
        }
        self.declared.insert(key.clone(), books);
    }
    fn remove_order(&mut self, key: &Arc<Key>, previous: &BookPartition, ranks: [Decimal; 3]) {
        if let Some(partition) = self.partitions.get_mut(previous) {
            Arc::make_mut(partition).remove(key, ranks);
            if partition.is_empty() {
                self.partitions.remove(previous);
            }
        }
        let book = &previous.book;
        if !self.active(book) {
            self.drop_book(book);
            let group = book.group();
            if let Some(books) = self.groups.get_mut(&group) {
                books.remove(book);
                if books.is_empty() {
                    self.groups.remove(&group);
                }
            }
        }
    }
    pub fn remove_feed(&mut self, key: &Key) {
        if let Some(books) = self.declared.remove(key) {
            for book in &books {
                self.drop_book(book);
            }
        }
    }
    pub fn remove(&mut self, key: &Key) -> Result<(), Status> {
        if let Some((identity, location)) = self
            .locations
            .get_key_value(key)
            .map(|(key, location)| (Arc::clone(key), Arc::clone(location)))
        {
            let stored = self
                .record(&location, key)
                .ok_or_else(|| Status::data_loss("missing located order"))?;
            let ranks = ranks(order(stored).ok_or_else(|| Status::data_loss("missing order"))?)?;
            self.remove_order(&identity, &location, ranks);
            self.locations.remove(key);
        }
        Ok(())
    }
    pub fn page(&self, after: Option<w::BookKey>, limit: usize) -> w::BooksPage {
        use std::ops::Bound::{Excluded, Unbounded};
        let after = after.as_ref().map(Book::from);
        let mut rows = self
            .references
            .range((after.map_or(Unbounded, Excluded), Unbounded));
        let books: Vec<w::BookKey> = rows
            .by_ref()
            .take(limit)
            .map(|(book, _)| book.clone().into())
            .collect();
        w::BooksPage {
            next_after: rows.next().and_then(|_| books.last().cloned()),
            books,
        }
    }
    /// Borrow records from the pinned index; merging never copies payloads or clones their Arcs.
    pub fn ranked<'a>(
        &'a self,
        scope: &Scope,
        after: Option<RankKey>,
        sort: Sort,
    ) -> RankedIterator<'a> {
        use std::ops::Bound::{Excluded, Included, Unbounded};
        let descending = sort.descending();
        let pair = BookGroup {
            item_type_id: scope.item_type_id.clone().unwrap_or_default(),
            currency_id: scope.currency_id.clone().unwrap_or_default(),
        };
        let mut iterators: Vec<RankedIterator<'a>> = Vec::new();
        let mut candidate_budget = crate::READ_LIMITS.max_scan_candidates;
        let books: Box<dyn Iterator<Item = &Book> + '_> = if scope.item_type_id.is_some() {
            Box::new(self.groups.get(&pair).into_iter().flatten())
        } else {
            Box::new(
                self.groups
                    .iter()
                    .filter(|(group, _)| {
                        scope
                            .currency_id
                            .as_ref()
                            .is_none_or(|id| id == &group.currency_id)
                    })
                    .flat_map(|(_, books)| books.iter()),
            )
        };
        for b in books {
            let scoped;
            let scope = if let Some(collections) = &scope.collection_attributes {
                let Some(attributes) = collections.get(&b.item_type_id) else {
                    continue;
                };
                scoped = {
                    let mut copy = scope.clone();
                    copy.attributes = attributes.clone();
                    copy.collection_attributes = None;
                    copy
                };
                &scoped
            } else {
                scope
            };
            if scope
                .venue_id
                .as_ref()
                .is_some_and(|venue| venue != &b.venue_id)
            {
                continue;
            }
            for mode in [ActiveMode::Ask, ActiveMode::Bid]
                .into_iter()
                .filter(|m| scope.modes.is_empty() || scope.modes.contains(&(*m).into()))
            {
                if let Some(partition) = self.partitions.get(&BookPartition {
                    book: b.clone(),
                    mode,
                }) {
                    let (min, max) = if sort.is_price() {
                        (
                            scope.price.as_ref().and_then(|p| p.min),
                            scope.price.as_ref().and_then(|p| p.max),
                        )
                    } else if scope.attribute_match == crate::AttributeMatch::Value
                        && matches!(sort, Sort::NumberAsc | Sort::NumberDesc)
                    {
                        number_bounds(scope.attributes.get("tg.number"))
                    } else {
                        (None, None)
                    };
                    let lower = min.map(RankKey::first);
                    let upper = max.map(RankKey::last);
                    let low = if !descending
                        && after
                            .as_ref()
                            .is_some_and(|a| lower.as_ref().is_none_or(|b| a >= b))
                    {
                        after.clone().map(Excluded).unwrap_or(Unbounded)
                    } else {
                        lower.map(Included).unwrap_or(Unbounded)
                    };
                    let high = if descending
                        && after
                            .as_ref()
                            .is_some_and(|a| upper.as_ref().is_none_or(|b| a <= b))
                    {
                        after.clone().map(Excluded).unwrap_or(Unbounded)
                    } else {
                        upper.map(Included).unwrap_or(Unbounded)
                    };
                    if let (Included(a) | Excluded(a), Included(b) | Excluded(b)) = (&low, &high)
                        && a >= b
                    {
                        continue;
                    }
                    if mode == ActiveMode::Ask
                        && let Some((count, sets)) =
                            partition.attributes.candidates(scope, candidate_budget)
                    {
                        candidate_budget -= count;
                        let mut rows: Vec<RankedOrder<'a>> = Vec::with_capacity(count);
                        for id in sets.into_iter().flatten() {
                            let entity = &partition.orders[&id.id];
                            let rank = sort
                                .rank(order(entity).expect("indexed order"), false)
                                .expect("validated order rank");
                            let lookup = RankKey::shared(rank, Arc::clone(id));
                            let (key, entity) = partition
                                .index(sort)
                                .get_key_value(&lookup)
                                .expect("covering rank index");
                            let after_low = match &low {
                                Included(v) => key >= v,
                                Excluded(v) => key > v,
                                Unbounded => true,
                            };
                            let before_high = match &high {
                                Included(v) => key <= v,
                                Excluded(v) => key < v,
                                Unbounded => true,
                            };
                            if after_low && before_high {
                                rows.push((key, entity));
                            }
                        }
                        rows.sort_unstable_by(|a, b| {
                            if descending {
                                b.0.cmp(a.0)
                            } else {
                                a.0.cmp(b.0)
                            }
                        });
                        iterators.push(Box::new(rows.into_iter()));
                        continue;
                    }
                    let range = partition.index(sort).range((low, high));
                    let iter: RankedIterator<'a> = if descending {
                        Box::new(range.rev())
                    } else {
                        Box::new(range)
                    };
                    let iter = iter.filter(move |(key, _)| {
                        min.is_none_or(|v| key.value >= v) && max.is_none_or(|v| key.value <= v)
                    });
                    iterators.push(Box::new(iter) as RankedIterator<'a>);
                }
            }
        }
        if descending {
            merge(iterators, |rank| rank)
        } else {
            merge(iterators, Reverse)
        }
    }
}

#[cfg(test)]
mod tests;
