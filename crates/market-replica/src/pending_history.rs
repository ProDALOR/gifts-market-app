//! Immutable execution tail: reads never wait for the durable writer.
use crate::{
    query::Filter,
    ranking::ordered_decimal,
    state,
    storage::{Snapshot, failure},
    types::{Position, Sort},
};
use market_hub_protocol::wire as w;
use std::{
    ops::Bound::{Excluded, Included},
    sync::Arc,
};
use tonic::Status;

type Rank = (u8, String, String, Vec<u8>, Position);
type Ranked = ([u8; 58], Position);
#[derive(Clone)]
struct Execution {
    sequence: u64,
    order: Arc<w::Order>,
}
#[derive(Clone, Default)]
pub(super) struct Pending {
    orders: im::OrdMap<Position, Execution>,
    identities: im::OrdMap<(String, String), Position>,
    ranks: im::OrdMap<Rank, ()>,
}
impl Snapshot {
    pub(crate) fn stage<'a>(
        &self,
        sequence: u64,
        changes: impl Iterator<Item = &'a w::Change>,
    ) -> Result<(Self, bool), Status> {
        let mut next = self.clone();
        let mut changed = false;
        for change in changes {
            let Some(w::change::Change::Upsert(entity)) = &change.change else {
                continue;
            };
            let Some(order) =
                state::order(entity).filter(|order| order.mode == w::OrderMode::Executed as i32)
            else {
                continue;
            };
            let book = order
                .book
                .as_ref()
                .ok_or_else(|| Status::data_loss("missing execution book"))?;
            let identity = (book.venue_id.clone(), order.id.clone());
            let position = Position {
                executed_at_ms: order
                    .executed_at_ms
                    .ok_or_else(|| Status::data_loss("missing execution time"))?,
                venue_id: book.venue_id.clone(),
                order_id: order.id.clone(),
            };
            let existing = if let Some(position) = next.pending.identities.get(&identity) {
                Some(
                    next.pending
                        .orders
                        .get(position)
                        .expect("pending identity")
                        .order
                        .as_ref()
                        .clone(),
                )
            } else {
                let identities = self
                    .read
                    .open_table(crate::storage::IDENTITIES)
                    .map_err(failure)?;
                match identities
                    .get((book.venue_id.as_str(), order.id.as_str()))
                    .map_err(failure)?
                {
                    Some(time) => self
                        .execution(&Position {
                            executed_at_ms: time.value(),
                            ..position.clone()
                        })?
                        .and_then(|entity| match entity.entity {
                            Some(w::entity::Entity::Order(order)) => Some(order),
                            _ => None,
                        }),
                    None => None,
                }
            };
            if let Some(mut existing) = existing {
                existing.observed_at_ms = order.observed_at_ms;
                if existing != *order {
                    return Err(Status::data_loss("executed identity changed contents"));
                }
                continue;
            }
            for sort in [Sort::PriceAsc, Sort::TimeAsc, Sort::NumberAsc] {
                next.pending.ranks.insert(
                    (
                        sort.criterion(),
                        book.item_type_id.clone(),
                        book.currency_id.clone(),
                        ordered_decimal(sort.rank(order, true)?),
                        position.clone(),
                    ),
                    (),
                );
            }
            next.pending.identities.insert(identity, position.clone());
            next.pending.orders.insert(
                position,
                Execution {
                    sequence,
                    order: Arc::new(order.clone()),
                },
            );
            changed = true;
        }
        Ok((next, changed))
    }
    pub(crate) fn persisted(&self, base: &Self, sequence: u64) -> Result<Self, Status> {
        let mut pending = self.pending.clone();
        for (position, entry) in &self.pending.orders {
            if entry.sequence <= sequence {
                pending.orders.remove(position);
                pending
                    .identities
                    .remove(&(position.venue_id.clone(), position.order_id.clone()));
                let book = entry.order.book.as_ref().expect("validated book");
                for sort in [Sort::PriceAsc, Sort::TimeAsc, Sort::NumberAsc] {
                    pending.ranks.remove(&(
                        sort.criterion(),
                        book.item_type_id.clone(),
                        book.currency_id.clone(),
                        ordered_decimal(sort.rank(&entry.order, true)?),
                        position.clone(),
                    ));
                }
            }
        }
        Ok(Self {
            read: base.read.clone(),
            _store: base._store.clone(),
            pending,
        })
    }
}
impl Pending {
    pub fn order(&self, position: &Position) -> Option<&Arc<w::Order>> {
        self.orders.get(position).map(|entry| &entry.order)
    }
    pub fn chronological<'a>(
        &'a self,
        after: Option<&'a Position>,
        from: i64,
        until: Option<i64>,
    ) -> impl Iterator<Item = Result<Position, Status>> + 'a {
        let start = after.cloned().map(Excluded).unwrap_or_else(|| {
            Included(Position {
                executed_at_ms: from,
                venue_id: String::new(),
                order_id: String::new(),
            })
        });
        let end = until
            .map(|time| {
                Excluded(Position {
                    executed_at_ms: time,
                    venue_id: String::new(),
                    order_id: String::new(),
                })
            })
            .unwrap_or(std::ops::Bound::Unbounded);
        self.orders
            .range((start, end))
            .filter(move |(position, _)| position.executed_at_ms >= from)
            .map(|(position, _)| Ok(position.clone()))
    }
    pub fn ranked<'a>(
        &'a self,
        filter: &Filter,
        after: Option<&(String, Position)>,
        end: Option<&(String, Position)>,
    ) -> Result<Box<dyn Iterator<Item = Result<Ranked, Status>> + 'a>, Status> {
        let scope = filter
            .scope
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing ranked scope"))?;
        let item = scope
            .item_type_id
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing item"))?;
        let currency = scope
            .currency_id
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing currency"))?;
        let key = |rank: Vec<u8>, position| {
            (
                filter.sort.criterion(),
                item.clone(),
                currency.clone(),
                rank,
                position,
            )
        };
        let boundary = |value: &(String, Position)| {
            Ok::<_, Status>(key(
                ordered_decimal(state::decimal(&value.0)?),
                value.1.clone(),
            ))
        };
        let after = after.map(boundary).transpose()?;
        let end = end.map(boundary).transpose()?;
        let minimum = key(
            vec![0; 58],
            Position {
                executed_at_ms: i64::MIN,
                venue_id: String::new(),
                order_id: String::new(),
            },
        );
        let maximum = key(
            vec![255; 58],
            Position {
                executed_at_ms: i64::MAX,
                venue_id: "\u{10ffff}".into(),
                order_id: "\u{10ffff}".into(),
            },
        );
        let (lower, upper) = if filter.sort.descending() {
            (
                end.map(Included).unwrap_or(Included(minimum)),
                after.map(Excluded).unwrap_or(Included(maximum)),
            )
        } else {
            (
                after.map(Excluded).unwrap_or(Included(minimum)),
                end.map(Included).unwrap_or(Included(maximum)),
            )
        };
        let rows = self.ranks.range((lower, upper));
        let rows: Box<dyn Iterator<Item = _>> = if filter.sort.descending() {
            Box::new(rows.rev())
        } else {
            Box::new(rows)
        };
        Ok(Box::new(rows.map(|((_, _, _, rank, position), _)| {
            Ok((
                <[u8; 58]>::try_from(rank.as_slice()).map_err(failure)?,
                position.clone(),
            ))
        })))
    }
}
pub(super) fn merge<T: Ord>(
    stored: impl Iterator<Item = Result<T, Status>>,
    pending: impl Iterator<Item = Result<T, Status>>,
    descending: bool,
) -> impl Iterator<Item = Result<T, Status>> {
    let mut stored = stored.peekable();
    let mut pending = pending.peekable();
    std::iter::from_fn(move || match (stored.peek(), pending.peek()) {
        (None, _) => pending.next(),
        (_, None) | (Some(Err(_)), _) => stored.next(),
        (_, Some(Err(_))) => pending.next(),
        (Some(Ok(left)), Some(Ok(right))) => {
            let order = left.cmp(right);
            if order.is_eq() {
                stored.next();
                return pending.next();
            }
            if order.is_lt() != descending {
                stored.next()
            } else {
                pending.next()
            }
        }
    })
}
