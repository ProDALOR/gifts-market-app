//! Generic revision keys survive coalesced notifications without retaining an event log.
use crate::{
    ReplicaReader, ReplicaView,
    state::{self, State},
    types::{EntityKind, Key},
};
use im::OrdMap;
use market_hub_protocol::wire as w;
use std::collections::BTreeSet;
use tonic::Status;

type Book = (String, String, String);
type Scope = (String, String, Option<String>);
#[derive(Clone, Default)]
pub(crate) struct Revisions {
    pub instance: uuid::Uuid,
    pub sequence: u64,
    pub dataset: u64,
    pub catalog: u64,
    pub directory: u64,
    books: OrdMap<Scope, u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadRevision {
    instance: uuid::Uuid,
    dataset: u64,
    catalog: u64,
    data: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogRevision {
    instance: uuid::Uuid,
    dataset: u64,
    catalog: u64,
    directory: u64,
}
fn book(value: &w::BookKey) -> Book {
    (
        value.item_type_id.clone(),
        value.currency_id.clone(),
        value.venue_id.clone(),
    )
}
fn affected(entity: &w::Entity, books: &mut BTreeSet<Book>) {
    match &entity.entity {
        Some(w::entity::Entity::Order(order)) => {
            if let Some(key) = &order.book {
                books.insert(book(key));
            }
        }
        Some(w::entity::Entity::Feed(feed)) => books.extend(feed.books.iter().map(book)),
        _ => {}
    }
}
impl Revisions {
    pub fn replaced(previous: Option<&ReplicaView>) -> Self {
        let sequence = previous.map_or(1, |view| view.revisions.sequence + 1);
        Self {
            instance: previous.map_or_else(uuid::Uuid::new_v4, |view| view.revisions.instance),
            sequence,
            dataset: sequence,
            catalog: sequence,
            directory: sequence,
            books: OrdMap::new(),
        }
    }
    pub fn updated(
        &self,
        before: &State,
        after: &State,
        changes: &[w::Change],
    ) -> Result<Self, Status> {
        let mut next = self.clone();
        next.sequence += 1;
        let mut touched = BTreeSet::new();
        let mut orders = BTreeSet::new();
        for change in changes {
            let key = match &change.change {
                Some(w::change::Change::Upsert(value)) => state::entity_key(value)?,
                Some(w::change::Change::Remove(value)) => Key::try_from(
                    value
                        .key
                        .as_ref()
                        .ok_or_else(|| Status::data_loss("missing key"))?,
                )?,
                None => return Err(Status::data_loss("empty change")),
            };
            let old = before.entities.get(&key);
            let new = after.entities.get(&key);
            // Immutable execution retries and identical observations need no active-book invalidation.
            if old == new {
                continue;
            }
            if key.kind != EntityKind::Order && key.kind != EntityKind::Feed {
                next.catalog = next.sequence;
            }
            if let Some(entity) = old {
                affected(entity, &mut touched);
            }
            if let Some(entity) = new {
                affected(entity, &mut touched);
            }
            if key.kind == EntityKind::Order {
                for entity in old.into_iter().chain(new) {
                    affected(entity, &mut orders);
                }
            }
        }
        for (item, currency, venue) in touched {
            let key = w::BookKey {
                item_type_id: item.clone(),
                currency_id: currency.clone(),
                venue_id: venue.clone(),
            };
            if before.books.contains(&key) != after.books.contains(&key) {
                next.directory = next.sequence;
            }
            if !orders.contains(&(item.clone(), currency.clone(), venue.clone())) {
                continue;
            }
            let specific = (item.clone(), currency.clone(), Some(venue));
            if after.books.has_orders(&key) {
                next.books.insert(specific, next.sequence);
            } else {
                next.books.remove(&specific);
            }
            let group = (item.clone(), currency.clone(), None);
            if after.books.has_group(&item, &currency) {
                next.books.insert(group, next.sequence);
            } else {
                next.books.remove(&group);
            }
        }
        Ok(next)
    }
}
impl ReadRevision {
    pub fn token(self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.instance, self.dataset, self.catalog, self.data
        )
    }
}
impl ReplicaReader {
    pub fn read_revision(&self, request: &w::ReadRequest) -> Result<ReadRevision, Status> {
        let view = self.view()?;
        let revisions = &view.revisions;
        let data = match request
            .selection
            .as_ref()
            .and_then(|selection| selection.source.as_ref())
        {
            Some(w::selection::Source::Scope(scope))
                if request
                    .selection
                    .as_ref()
                    .is_none_or(|selection| selection.executed_at.is_none())
                    && !scope.modes.contains(&(w::OrderMode::Executed as i32)) =>
            {
                match (&scope.item_type_id, &scope.currency_id) {
                    (Some(item), Some(currency)) => revisions
                        .books
                        .get(&(item.clone(), currency.clone(), scope.venue_id.clone()))
                        .copied()
                        .unwrap_or_else(|| {
                            let present = scope.venue_id.as_ref().map_or_else(
                                || view.state.books.has_group(item, currency),
                                |venue| {
                                    view.state.books.has_orders(&w::BookKey {
                                        item_type_id: item.clone(),
                                        currency_id: currency.clone(),
                                        venue_id: venue.clone(),
                                    })
                                },
                            );
                            if present { revisions.dataset } else { 0 }
                        }),
                    _ => revisions.sequence,
                }
            }
            _ => revisions.sequence,
        };
        Ok(ReadRevision {
            instance: revisions.instance,
            dataset: revisions.dataset,
            catalog: revisions.catalog,
            data,
        })
    }
    pub fn catalog_revision(&self) -> Result<CatalogRevision, Status> {
        let view = self.view()?;
        Ok(CatalogRevision {
            instance: view.revisions.instance,
            dataset: view.revisions.dataset,
            catalog: view.revisions.catalog,
            directory: view.revisions.directory,
        })
    }
    pub fn cursor(&self) -> Result<String, Status> {
        Ok(self.view()?.cursor.clone())
    }
}
