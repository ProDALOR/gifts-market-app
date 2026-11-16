//! Derived book directory and active-order price indexes, published with the entity snapshot.
use crate::{
    query::Scope,
    state::{Key, decimal, order},
    types::{ActiveMode, PriceKey, Sort},
};
use im::{OrdMap, OrdSet};
use market_hub_protocol::wire as w;
use std::{cmp::Reverse, collections::BinaryHeap};
use tonic::Status;

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

#[derive(Clone, Default)]
pub struct BookIndex {
    prices: OrdMap<BookPartition, OrdSet<PriceKey>>,
    groups: OrdMap<BookGroup, OrdSet<Book>>,
    declared: OrdMap<Key, OrdSet<Book>>,
    references: OrdMap<Book, usize>,
}
impl BookIndex {
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
            self.prices.contains_key(&BookPartition {
                book: book.clone(),
                mode,
            })
        })
    }
    pub fn insert(&mut self, key: &Key, entity: &w::Entity) -> Result<(), Status> {
        if let Some(order) = order(entity) {
            let book = Book::from(
                order
                    .book
                    .as_ref()
                    .ok_or_else(|| Status::data_loss("missing book"))?,
            );
            if !self.active(&book) {
                self.add_book(book.clone());
            }
            self.prices
                .entry(BookPartition {
                    book: book.clone(),
                    mode: ActiveMode::try_from(order.mode)?,
                })
                .or_default()
                .insert(PriceKey::new(decimal(&order.price)?, key.clone()));
            self.groups.entry(book.group()).or_default().insert(book);
        }
        if let Some(w::entity::Entity::Feed(feed)) = &entity.entity {
            let books: OrdSet<Book> = feed.books.iter().map(Book::from).collect();
            for book in &books {
                self.add_book(book.clone());
            }
            self.declared.insert(key.clone(), books);
        }
        Ok(())
    }
    pub fn remove(&mut self, key: &Key, entity: &w::Entity) -> Result<(), Status> {
        if let Some(order) = order(entity) {
            let book = Book::from(
                order
                    .book
                    .as_ref()
                    .ok_or_else(|| Status::data_loss("missing book"))?,
            );
            let partition = BookPartition {
                book: book.clone(),
                mode: ActiveMode::try_from(order.mode)?,
            };
            if let Some(keys) = self.prices.get_mut(&partition) {
                keys.remove(&PriceKey::new(decimal(&order.price)?, key.clone()));
                if keys.is_empty() {
                    self.prices.remove(&partition);
                }
            }
            if !self.active(&book) {
                self.drop_book(&book);
                let group = book.group();
                if let Some(books) = self.groups.get_mut(&group) {
                    books.remove(&book);
                    if books.is_empty() {
                        self.groups.remove(&group);
                    }
                }
            }
        }
        if let Some(books) = self.declared.remove(key) {
            for book in &books {
                self.drop_book(book);
            }
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
    pub fn prices<'a>(
        &'a self,
        scope: &Scope,
        after: Option<PriceKey>,
        sort: Sort,
    ) -> Box<dyn Iterator<Item = PriceKey> + 'a> {
        use std::ops::Bound::{Excluded, Included, Unbounded};
        let descending = sort == Sort::PriceDesc;
        let pair = BookGroup {
            item_type_id: scope.item_type_id.clone().unwrap_or_default(),
            currency_id: scope.currency_id.clone().unwrap_or_default(),
        };
        let mut iterators: Vec<Box<dyn Iterator<Item = &PriceKey> + 'a>> = Vec::new();
        for b in self.groups.get(&pair).into_iter().flatten() {
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
                if let Some(keys) = self.prices.get(&BookPartition {
                    book: b.clone(),
                    mode,
                }) {
                    let min = scope.price.as_ref().and_then(|p| p.min);
                    let max = scope.price.as_ref().and_then(|p| p.max);
                    let lower = min.map(PriceKey::first);
                    let upper = max.map(PriceKey::last);
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
                    let range = keys.range((low, high));
                    let iter: Box<dyn Iterator<Item = &PriceKey> + 'a> = if descending {
                        Box::new(range.rev())
                    } else {
                        Box::new(range)
                    };
                    let iter = iter.filter(move |key| {
                        min.is_none_or(|v| key.price >= v) && max.is_none_or(|v| key.price <= v)
                    });
                    iterators.push(Box::new(iter));
                }
            }
        }
        if descending {
            let mut heap = BinaryHeap::new();
            for (i, it) in iterators.iter_mut().enumerate() {
                if let Some(value) = it.next() {
                    heap.push((value.clone(), i));
                }
            }
            Box::new(std::iter::from_fn(move || {
                let (value, i) = heap.pop()?;
                if let Some(next) = iterators[i].next() {
                    heap.push((next.clone(), i));
                }
                Some(value)
            }))
        } else {
            let mut heap = BinaryHeap::new();
            for (i, it) in iterators.iter_mut().enumerate() {
                if let Some(value) = it.next() {
                    heap.push(Reverse((value.clone(), i)));
                }
            }
            Box::new(std::iter::from_fn(move || {
                let Reverse((value, i)) = heap.pop()?;
                if let Some(next) = iterators[i].next() {
                    heap.push(Reverse((next.clone(), i)));
                }
                Some(value)
            }))
        }
    }
}
