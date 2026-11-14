use hub_protocol::wire as w;
use im::{OrdMap, OrdSet};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{cmp::Reverse, collections::BinaryHeap, sync::Arc};
use tonic::Status;

pub type Book = (String, String, String);
pub type PriceKey = (Decimal, Key);
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Key(pub i32, pub String, pub String);
impl Key {
    pub fn wire(value: &w::Key) -> Self {
        Self(value.kind, value.venue_id.clone(), value.id.clone())
    }
}
pub fn book(value: &w::BookKey) -> Book {
    (
        value.venue_id.clone(),
        value.item_type_id.clone(),
        value.currency_id.clone(),
    )
}
pub fn entity_key(entity: &w::Entity) -> Result<Key, Status> {
    use w::entity::Entity as E;
    let (kind, id, venue) = match entity
        .entity
        .as_ref()
        .ok_or_else(|| Status::data_loss("empty entity"))?
    {
        E::Venue(v) => (1, &v.id, ""),
        E::Currency(v) => (2, &v.id, ""),
        E::ItemType(v) => (3, &v.id, ""),
        E::Attribute(v) => (4, &v.id, ""),
        E::AttributeOption(v) => (5, &v.id, ""),
        E::Feed(v) => (6, &v.id, ""),
        E::Order(v) => (
            7,
            &v.id,
            v.book
                .as_ref()
                .ok_or_else(|| Status::data_loss("missing order book"))?
                .venue_id
                .as_str(),
        ),
    };
    Ok(Key(kind, venue.into(), id.clone()))
}
pub fn order(entity: &w::Entity) -> Option<&w::Order> {
    match entity.entity.as_ref()? {
        w::entity::Entity::Order(order) => Some(order),
        _ => None,
    }
}
pub fn decimal(value: &str) -> Result<Decimal, Status> {
    if value.len() > 128 {
        return Err(Status::invalid_argument("decimal too long"));
    }
    Decimal::from_str_exact(value)
        .map(|v| v.normalize())
        .map_err(|_| Status::invalid_argument("invalid exact decimal"))
}
#[derive(Clone, Default)]
pub struct State {
    pub entities: OrdMap<Key, Arc<w::Entity>>,
    prices: OrdMap<(Book, i32), OrdSet<PriceKey>>,
    pub groups: OrdMap<(String, String), OrdSet<Book>>,
    pub declared: OrdMap<Key, OrdSet<Book>>,
    book_counts: OrdMap<Book, usize>,
}
impl State {
    pub fn apply(&mut self, changes: &[w::Change]) -> Result<(), Status> {
        for change in changes {
            match &change.change {
                Some(w::change::Change::Upsert(entity)) => self.upsert(entity.clone())?,
                Some(w::change::Change::Remove(remove)) => self.remove(&Key::wire(
                    remove
                        .key
                        .as_ref()
                        .ok_or_else(|| Status::data_loss("missing removal key"))?,
                ))?,
                None => return Err(Status::data_loss("empty change")),
            }
        }
        Ok(())
    }
    fn add_book(&mut self, book: Book) {
        *self.book_counts.entry(book).or_default() += 1;
    }
    fn drop_book(&mut self, book: &Book) {
        if let Some(count) = self.book_counts.get_mut(book) {
            *count -= 1;
            if *count == 0 {
                self.book_counts.remove(book);
            }
        }
    }
    pub fn get(&self, kind: i32, id: &str) -> Option<&w::entity::Entity> {
        self.entities
            .get(&Key(kind, String::new(), id.into()))?
            .entity
            .as_ref()
    }
    pub fn upsert(&mut self, entity: w::Entity) -> Result<(), Status> {
        let key = entity_key(&entity)?;
        self.remove(&key)?;
        if let Some(order) = order(&entity) {
            if order.mode == w::OrderMode::Executed as i32 {
                return Ok(());
            }
            let b = book(
                order
                    .book
                    .as_ref()
                    .ok_or_else(|| Status::data_loss("missing book"))?,
            );
            if ![1, 2]
                .iter()
                .any(|mode| self.prices.contains_key(&(b.clone(), *mode)))
            {
                self.add_book(b.clone());
            }
            self.prices
                .entry((b.clone(), order.mode))
                .or_default()
                .insert((decimal(&order.price)?, key.clone()));
            self.groups
                .entry((b.1.clone(), b.2.clone()))
                .or_default()
                .insert(b);
        }
        if let Some(w::entity::Entity::Feed(feed)) = &entity.entity {
            let books: OrdSet<Book> = feed.books.iter().map(book).collect();
            for book in &books {
                self.add_book(book.clone());
            }
            self.declared.insert(key.clone(), books);
        }
        self.entities.insert(key, Arc::new(entity));
        Ok(())
    }
    pub fn remove(&mut self, key: &Key) -> Result<(), Status> {
        let Some(entity) = self.entities.remove(key) else {
            return Ok(());
        };
        if let Some(order) = order(&entity) {
            let b = book(
                order
                    .book
                    .as_ref()
                    .ok_or_else(|| Status::data_loss("missing book"))?,
            );
            let partition = (b.clone(), order.mode);
            if let Some(keys) = self.prices.get_mut(&partition) {
                keys.remove(&(decimal(&order.price)?, key.clone()));
                if keys.is_empty() {
                    self.prices.remove(&partition);
                }
            }
            if ![1, 2]
                .iter()
                .any(|mode| self.prices.contains_key(&(b.clone(), *mode)))
            {
                self.drop_book(&b);
                let group = (b.1.clone(), b.2.clone());
                if let Some(books) = self.groups.get_mut(&group) {
                    books.remove(&b);
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
    pub fn books(&self) -> &OrdMap<Book, usize> {
        &self.book_counts
    }
    pub fn prices<'a>(
        &'a self,
        scope: &w::Scope,
        after: Option<PriceKey>,
        descending: bool,
    ) -> Box<dyn Iterator<Item = PriceKey> + 'a> {
        use std::ops::Bound::{Excluded, Included, Unbounded};
        let pair = (
            scope.item_type_id.clone().unwrap_or_default(),
            scope.currency_id.clone().unwrap_or_default(),
        );
        let mut iterators: Vec<Box<dyn Iterator<Item = &PriceKey> + 'a>> = Vec::new();
        for b in self.groups.get(&pair).into_iter().flatten() {
            if scope.venue_id.as_ref().is_some_and(|venue| venue != &b.0) {
                continue;
            }
            for mode in [1, 2]
                .into_iter()
                .filter(|m| scope.modes.is_empty() || scope.modes.contains(m))
            {
                if let Some(keys) = self.prices.get(&(b.clone(), mode)) {
                    let min = scope
                        .price
                        .as_ref()
                        .and_then(|p| p.min.as_ref())
                        .and_then(|v| decimal(v).ok());
                    let max = scope
                        .price
                        .as_ref()
                        .and_then(|p| p.max.as_ref())
                        .and_then(|v| decimal(v).ok());
                    let lower = min.map(|v| (v, Key(0, String::new(), String::new())));
                    let upper = max.map(|v| (v, Key(8, String::new(), String::new())));
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
                    let iter = iter.filter(move |(price, _)| {
                        min.is_none_or(|v| *price >= v) && max.is_none_or(|v| *price <= v)
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
