use crate::{
    query::Filter,
    state::{self, State},
    types::{IndexKind, IndexTerm, Key, Leaf, OrderMode, Sort},
};
use market_hub_protocol::wire as w;
use prost::Message;
use redb::{
    Database, Durability, ReadTransaction, ReadableDatabase, ReadableTable, ReadableTableMetadata,
    TableDefinition, WriteTransaction,
};
use std::{collections::BTreeSet, path::Path, sync::Arc};
use tonic::Status;

pub use crate::types::Position;
type Identity<'a> = (i32, &'a str, &'a str);
type TimeKey<'a> = (i64, &'a str, &'a str);
type IndexKey<'a> = (u8, &'a [u8], &'a [u8], TimeKey<'a>);
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata");
const CURRENT: TableDefinition<Identity<'static>, &[u8]> = TableDefinition::new("current");
const EXECUTIONS: TableDefinition<TimeKey<'static>, &[u8]> = TableDefinition::new("executions");
const IDENTITIES: TableDefinition<(&str, &str), i64> = TableDefinition::new("identities");
const INDEX: TableDefinition<IndexKey<'static>, ()> = TableDefinition::new("index");
pub(crate) type RankedHistoryPage = (Vec<w::Entity>, Option<(String, Position)>);
type RankedExecution<'a> = (u8, &'a str, &'a str, &'a [u8], TimeKey<'a>);
const SORTED: TableDefinition<RankedExecution<'static>, ()> =
    TableDefinition::new("execution_ranks_v1");
const JOURNAL: TableDefinition<u64, &[u8]> = TableDefinition::new("bootstrap_journal");
#[derive(Clone)]
pub struct Store(Arc<Database>, #[cfg(test)] Arc<std::sync::atomic::AtomicU8>);
#[derive(Clone)]
pub struct Snapshot {
    pub read: Arc<ReadTransaction>,
    pub _store: Store,
}
pub(crate) struct Applied {
    pub updated_at: Option<i64>,
    pub data_changed: bool,
    pub history_changed: bool,
}
#[derive(Debug)]
pub(crate) struct ApplyError {
    pub status: Status,
    pub commit_attempted: bool,
}
impl From<Status> for ApplyError {
    fn from(status: Status) -> Self {
        Self {
            status,
            commit_attempted: false,
        }
    }
}
impl From<ApplyError> for Status {
    fn from(error: ApplyError) -> Self {
        error.status
    }
}
pub(crate) struct Checkpoint {
    pub cursor: String,
    pub updated_at: Option<i64>,
}
pub fn failure(error: impl std::fmt::Display) -> Status {
    tracing::error!(%error,"replica storage failed");
    Status::unavailable("local market storage unavailable")
}
fn value(
    meta: &impl ReadableTable<&'static str, &'static [u8]>,
    key: &str,
) -> Result<Vec<u8>, Status> {
    Ok(meta
        .get(key)
        .map_err(failure)?
        .map(|v| v.value().to_vec())
        .unwrap_or_default())
}
impl Store {
    pub fn open(path: &Path, source: &str) -> Result<Self, Status> {
        std::fs::create_dir_all(
            path.parent()
                .ok_or_else(|| failure("missing data directory"))?,
        )
        .map_err(failure)?;
        let database = Database::create(path).map_err(failure)?;
        let tx = database.begin_write().map_err(failure)?;
        {
            let mut meta = tx.open_table(META).map_err(failure)?;
            let stored = value(&meta, "source")?;
            if !stored.is_empty() && stored != source.as_bytes() {
                return Err(Status::failed_precondition(
                    "data directory belongs to another hub or service identity",
                ));
            }
            meta.insert("source", source.as_bytes()).map_err(failure)?;
        }
        tx.open_table(CURRENT).map_err(failure)?;
        tx.open_table(EXECUTIONS).map_err(failure)?;
        tx.open_table(IDENTITIES).map_err(failure)?;
        tx.open_table(INDEX).map_err(failure)?;
        tx.open_table(JOURNAL).map_err(failure)?;
        {
            let mut meta = tx.open_table(META).map_err(failure)?;
            let mut ranked = tx.open_table(SORTED).map_err(failure)?;
            if value(&meta, "execution_ranks_v1")?.is_empty() {
                let executions = tx.open_table(EXECUTIONS).map_err(failure)?;
                for row in executions.iter().map_err(failure)? {
                    let (_, raw) = row.map_err(failure)?;
                    let order = w::Order::decode(raw.value()).map_err(failure)?;
                    insert_ranks(&mut ranked, &order)?;
                }
                meta.insert("execution_ranks_v1", b"1".as_slice())
                    .map_err(failure)?;
            }
        }
        tx.commit().map_err(failure)?;
        Ok(Self(
            Arc::new(database),
            #[cfg(test)]
            Arc::default(),
        ))
    }
    pub fn snapshot(&self) -> Result<Snapshot, Status> {
        Ok(Snapshot {
            read: Arc::new(self.0.begin_read().map_err(failure)?),
            _store: self.clone(),
        })
    }
    #[cfg(test)]
    pub fn fail_commit_once(&self, committed: bool) {
        self.1.store(
            if committed { 2 } else { 1 },
            std::sync::atomic::Ordering::SeqCst,
        );
    }
    pub fn cursor(&self) -> Result<String, Status> {
        Ok(self.snapshot()?.checkpoint()?.cursor)
    }
    #[cfg(test)]
    pub fn updated_at(&self) -> Result<Option<i64>, Status> {
        Ok(self.snapshot()?.checkpoint()?.updated_at)
    }
    #[cfg(test)]
    pub fn state(&self) -> Result<State, Status> {
        self.snapshot()?.state()
    }
    pub fn apply<'a>(
        &self,
        changes: impl IntoIterator<Item = &'a w::Change>,
        cursor: Option<&str>,
    ) -> Result<Applied, ApplyError> {
        let mut tx = self.0.begin_write().map_err(failure)?;
        tx.set_durability(if cursor.is_some() {
            Durability::Immediate
        } else {
            Durability::None
        })
        .map_err(failure)?;
        let mut applied = Applied {
            updated_at: None,
            data_changed: false,
            history_changed: false,
        };
        let mut changes = changes.into_iter().peekable();
        if changes.peek().is_some() {
            let mut current = tx.open_table(CURRENT).map_err(failure)?;
            let mut executions = tx.open_table(EXECUTIONS).map_err(failure)?;
            let mut identities = tx.open_table(IDENTITIES).map_err(failure)?;
            let mut index = tx.open_table(INDEX).map_err(failure)?;
            let mut ranked = tx.open_table(SORTED).map_err(failure)?;
            let mut encoded = Vec::new();
            for change in changes {
                match change
                    .change
                    .as_ref()
                    .ok_or_else(|| Status::data_loss("empty change"))?
                {
                    w::change::Change::Remove(remove) => {
                        let k = remove
                            .key
                            .as_ref()
                            .ok_or_else(|| Status::data_loss("missing removal key"))?;
                        applied.data_changed |= current
                            .remove(Key::try_from(k)?.stored())
                            .map_err(failure)?
                            .is_some();
                    }
                    w::change::Change::Upsert(entity) => {
                        let key = state::entity_key(entity)?;
                        if let Some(order) = state::order(entity).filter(|v| {
                            OrderMode::try_from(v.mode).ok() == Some(OrderMode::Executed)
                        }) {
                            let book = order
                                .book
                                .as_ref()
                                .ok_or_else(|| Status::data_loss("missing book"))?;
                            let venue = book.venue_id.as_str();
                            let id = order.id.as_str();
                            let time = order
                                .executed_at_ms
                                .ok_or_else(|| Status::data_loss("missing execution time"))?;
                            applied.data_changed |=
                                current.remove(key.stored()).map_err(failure)?.is_some();
                            if let Some(old) = identities.get((venue, id)).map_err(failure)? {
                                let payload = executions
                                    .get((old.value(), venue, id))
                                    .map_err(failure)?
                                    .ok_or_else(|| failure("missing execution identity"))?;
                                let mut stored =
                                    w::Order::decode(payload.value()).map_err(failure)?;
                                stored.observed_at_ms = order.observed_at_ms;
                                if stored != *order {
                                    return Err(Status::data_loss(
                                        "executed identity changed contents",
                                    )
                                    .into());
                                }
                                continue;
                            }
                            encoded.clear();
                            encoded.reserve(order.encoded_len());
                            order.encode(&mut encoded).map_err(failure)?;
                            executions
                                .insert((time, venue, id), encoded.as_slice())
                                .map_err(failure)?;
                            applied.data_changed = true;
                            applied.history_changed = true;
                            identities.insert((venue, id), time).map_err(failure)?;
                            insert_ranks(&mut ranked, order)?;
                            for IndexTerm { kind, path, value } in execution_indexes(order)? {
                                index
                                    .insert(
                                        (
                                            kind as u8,
                                            path.as_slice(),
                                            value.as_slice(),
                                            (time, venue, id),
                                        ),
                                        (),
                                    )
                                    .map_err(failure)?;
                            }
                        } else {
                            encoded.clear();
                            encoded.reserve(entity.encoded_len());
                            entity.encode(&mut encoded).map_err(failure)?;
                            let identical = current
                                .get(key.stored())
                                .map_err(failure)?
                                .is_some_and(|old| old.value() == encoded.as_slice());
                            if !identical {
                                current
                                    .insert(key.stored(), encoded.as_slice())
                                    .map_err(failure)?;
                                applied.data_changed = true;
                            }
                        }
                    }
                }
            }
        }
        if let Some(cursor) = cursor {
            applied.updated_at = Some(checkpoint(&tx, cursor)?);
        }
        #[cfg(test)]
        let fault = self.1.swap(0, std::sync::atomic::Ordering::SeqCst);
        #[cfg(test)]
        if fault == 1 {
            return Err(ApplyError {
                status: Status::unavailable("test uncertain commit"),
                commit_attempted: true,
            });
        }
        tx.commit().map_err(|error| ApplyError {
            status: failure(error),
            commit_attempted: true,
        })?;
        #[cfg(test)]
        if fault == 2 {
            return Err(ApplyError {
                status: Status::unavailable("test uncertain commit"),
                commit_attempted: true,
            });
        }
        Ok(applied)
    }
    pub fn journal(&self, batch: &w::Batch) -> Result<(), Status> {
        let mut tx = self.0.begin_write().map_err(failure)?;
        tx.set_durability(Durability::None).map_err(failure)?;
        {
            let mut table = tx.open_table(JOURNAL).map_err(failure)?;
            let next = table.len().map_err(failure)?;
            table
                .insert(next, batch.encode_to_vec().as_slice())
                .map_err(failure)?;
        }
        tx.commit().map_err(failure)
    }
    pub fn journal_range(
        &self,
        position: u64,
        max_batches: usize,
        max_bytes: usize,
    ) -> Result<Vec<w::Batch>, Status> {
        let snap = self.snapshot()?;
        let table = snap.read.open_table(JOURNAL).map_err(failure)?;
        let mut batches = Vec::new();
        let mut bytes: usize = 0;
        for row in table.range(position..).map_err(failure)? {
            let (key, raw) = row.map_err(failure)?;
            if batches.len() == max_batches
                || (!batches.is_empty() && raw.value().len() > max_bytes.saturating_sub(bytes))
            {
                break;
            }
            if key.value() != position + batches.len() as u64 {
                return Err(Status::data_loss("snapshot journal has a gap"));
            }
            if raw.value().len() > max_bytes.saturating_sub(bytes) {
                return Err(Status::resource_exhausted(
                    "snapshot journal delivery exceeds replay budget",
                ));
            }
            bytes += raw.value().len();
            batches.push(w::Batch::decode(raw.value()).map_err(failure)?);
        }
        Ok(batches)
    }
    pub fn finish(&self, cursor: &str) -> Result<(), Status> {
        let mut tx = self.0.begin_write().map_err(failure)?;
        tx.set_durability(Durability::Immediate).map_err(failure)?;
        checkpoint(&tx, cursor)?;
        tx.delete_table(JOURNAL).map_err(failure)?;
        tx.open_table(JOURNAL).map_err(failure)?;
        tx.commit().map_err(failure)
    }
}
fn checkpoint(tx: &WriteTransaction, cursor: &str) -> Result<i64, Status> {
    if cursor.is_empty() {
        return Err(Status::data_loss("empty durable checkpoint"));
    }
    let mut meta = tx.open_table(META).map_err(failure)?;
    meta.insert("cursor", cursor.as_bytes()).map_err(failure)?;
    let updated_at = crate::now();
    meta.insert("updated_at", updated_at.to_string().as_bytes())
        .map_err(failure)?;
    Ok(updated_at)
}
fn insert_ranks(
    table: &mut redb::Table<'_, RankedExecution<'static>, ()>,
    order: &w::Order,
) -> Result<(), Status> {
    let book = order
        .book
        .as_ref()
        .ok_or_else(|| Status::data_loss("missing book"))?;
    let time = order
        .executed_at_ms
        .ok_or_else(|| Status::data_loss("missing execution time"))?;
    for sort in [Sort::PriceAsc, Sort::TimeAsc, Sort::NumberAsc] {
        let rank = crate::ranking::ordered_decimal(sort.rank(order, true)?);
        table
            .insert(
                (
                    sort.criterion(),
                    book.item_type_id.as_str(),
                    book.currency_id.as_str(),
                    rank.as_slice(),
                    (time, book.venue_id.as_str(), order.id.as_str()),
                ),
                (),
            )
            .map_err(failure)?;
    }
    Ok(())
}
pub fn leaves(
    path: &mut Vec<String>,
    value: &w::AttributeValue,
    out: &mut BTreeSet<Leaf>,
) -> Result<(), Status> {
    use w::attribute_value::Value as V;
    match value
        .value
        .as_ref()
        .ok_or_else(|| Status::data_loss("empty attribute"))?
    {
        V::List(values) => {
            for value in &values.values {
                leaves(path, value, out)?;
            }
        }
        V::Object(fields) => {
            for (key, value) in &fields.fields {
                path.push(key.clone());
                leaves(path, value, out)?;
                path.pop();
            }
        }
        _ => {
            let mut value = value.clone();
            if let Some(V::Decimal(v)) = &mut value.value {
                *v = state::decimal(v)?.to_string();
            }
            out.insert(Leaf {
                path: serde_json::to_vec(path).map_err(failure)?,
                value: value.encode_to_vec(),
            });
        }
    }
    Ok(())
}
fn execution_indexes(order: &w::Order) -> Result<BTreeSet<IndexTerm>, Status> {
    let b = order
        .book
        .as_ref()
        .ok_or_else(|| Status::data_loss("missing book"))?;
    let mut rows: BTreeSet<_> = [
        IndexTerm {
            kind: IndexKind::Venue,
            path: vec![],
            value: b.venue_id.as_bytes().to_vec(),
        },
        IndexTerm {
            kind: IndexKind::ItemType,
            path: vec![],
            value: b.item_type_id.as_bytes().to_vec(),
        },
        IndexTerm {
            kind: IndexKind::Book,
            path: b.item_type_id.as_bytes().to_vec(),
            value: b.currency_id.as_bytes().to_vec(),
        },
    ]
    .into();
    for (id, predicate) in &order.attributes {
        if let Some(w::attribute_predicate::Predicate::Eq(value)) = &predicate.predicate {
            let mut facts = BTreeSet::new();
            leaves(&mut vec![id.clone()], value, &mut facts)?;
            rows.extend(facts.into_iter().map(|Leaf { path, value }| IndexTerm {
                kind: IndexKind::Attribute,
                path,
                value,
            }));
        }
    }
    Ok(rows)
}
impl Snapshot {
    pub fn checkpoint(&self) -> Result<Checkpoint, Status> {
        let meta = self.read.open_table(META).map_err(failure)?;
        let cursor = String::from_utf8(value(&meta, "cursor")?).map_err(failure)?;
        let raw = meta.get("updated_at").map_err(failure)?;
        let updated_at = match raw {
            Some(raw) if !raw.value().is_empty() => Some(
                std::str::from_utf8(raw.value())
                    .map_err(failure)?
                    .parse()
                    .map_err(failure)?,
            ),
            _ => None,
        };
        Ok(Checkpoint { cursor, updated_at })
    }
    pub fn state(&self) -> Result<State, Status> {
        let table = self.read.open_table(CURRENT).map_err(failure)?;
        let mut state = State::default();
        for row in table.iter().map_err(failure)? {
            let (_, payload) = row.map_err(failure)?;
            state.upsert(w::Entity::decode(payload.value()).map_err(failure)?)?;
        }
        Ok(state)
    }
    pub fn ranked_history(
        &self,
        filter: &Filter,
        after: Option<(String, Position)>,
        limit: usize,
        include: &dyn Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<RankedHistoryPage, Status> {
        self.ranked_history_between(filter, after, None, limit, include)
    }
    pub fn execution(&self, position: &Position) -> Result<Option<w::Entity>, Status> {
        let table = self.read.open_table(EXECUTIONS).map_err(failure)?;
        table
            .get(position.stored())
            .map_err(failure)?
            .map(|raw| {
                let order = w::Order::decode(raw.value()).map_err(failure)?;
                Ok(w::Entity {
                    entity: Some(w::entity::Entity::Order(order)),
                })
            })
            .transpose()
    }
    /// Scan an indexed, closed upper boundary in the requested ordering direction.
    pub fn ranked_history_between(
        &self,
        filter: &Filter,
        after: Option<(String, Position)>,
        end: Option<(String, Position)>,
        limit: usize,
        include: &dyn Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<RankedHistoryPage, Status> {
        use std::ops::Bound::{Excluded, Included};
        let scope = filter
            .scope
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing ranked scope"))?;
        let item = scope
            .item_type_id
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("missing item"))?;
        let currency = scope
            .currency_id
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("missing currency"))?;
        let table = self.read.open_table(SORTED).map_err(failure)?;
        let executions = self.read.open_table(EXECUTIONS).map_err(failure)?;
        let criterion = filter.sort.criterion();
        let minimum = [0u8; 58];
        let maximum = [255u8; 58];
        let encoded = after
            .as_ref()
            .map(|(value, _)| state::decimal(value).map(crate::ranking::ordered_decimal))
            .transpose()?;
        let previous = encoded
            .as_ref()
            .zip(after.as_ref())
            .map(|(encoded, (_, position))| {
                (
                    criterion,
                    item,
                    currency,
                    encoded.as_slice(),
                    position.stored(),
                )
            });
        let end_encoded = end
            .as_ref()
            .map(|(value, _)| state::decimal(value).map(crate::ranking::ordered_decimal))
            .transpose()?;
        let end_key = end_encoded
            .as_ref()
            .zip(end.as_ref())
            .map(|(encoded, (_, position))| {
                (
                    criterion,
                    item,
                    currency,
                    encoded.as_slice(),
                    position.stored(),
                )
            });
        let lower = if filter.sort.descending() {
            end_key.map(Included)
        } else {
            None
        }
        .or_else(|| {
            if !filter.sort.descending() {
                previous.map(Excluded)
            } else {
                None
            }
        })
        .unwrap_or(Included((
            criterion,
            item,
            currency,
            minimum.as_slice(),
            (i64::MIN, "", ""),
        )));
        let upper = if !filter.sort.descending() {
            end_key.map(Included)
        } else {
            None
        }
        .or_else(|| {
            if filter.sort.descending() {
                previous.map(Excluded)
            } else {
                None
            }
        })
        .unwrap_or(Included((
            criterion,
            item,
            currency,
            maximum.as_slice(),
            (i64::MAX, "\u{10ffff}", "\u{10ffff}"),
        )));
        let range = table.range((lower, upper)).map_err(failure)?;
        let rows: Box<dyn Iterator<Item = _>> = if filter.sort.descending() {
            Box::new(range.rev())
        } else {
            Box::new(range)
        };
        let mut data = Vec::new();
        let mut bytes = 0;
        let mut position = after;
        for (count, row) in rows.enumerate() {
            if count == crate::READ_LIMITS.max_scan_candidates {
                return Ok((data, position));
            }
            let (key, _) = row.map_err(failure)?;
            let (_, _, _, _, (time, venue, id)) = key.value();
            let raw = executions
                .get((time, venue, id))
                .map_err(failure)?
                .ok_or_else(|| failure("missing indexed execution"))?;
            let order = w::Order::decode(raw.value()).map_err(failure)?;
            let rank = filter.sort.rank(&order, true)?.to_string();
            if filter.matches_order(&order)? {
                let entity = w::Entity {
                    entity: Some(w::entity::Entity::Order(order)),
                };
                if include(&entity)? {
                    if data.len() == limit {
                        return Ok((data, position));
                    }
                    let size = entity.encoded_len() + 8;
                    if bytes + size > crate::READ_LIMITS.max_read_bytes {
                        if data.is_empty() {
                            return Err(Status::resource_exhausted("order exceeds page budget"));
                        }
                        return Ok((data, position));
                    }
                    bytes += size;
                    data.push(entity);
                }
            }
            position = Some((
                rank,
                Position {
                    executed_at_ms: time,
                    venue_id: venue.into(),
                    order_id: id.into(),
                },
            ));
        }
        Ok((data, None))
    }
    /// Scan a necessary index in chronological order, checking the full predicate afterwards.
    pub fn history(
        &self,
        filter: &Filter,
        after: Option<&Position>,
        limit: usize,
        include: &dyn Fn(&w::Entity) -> Result<bool, Status>,
    ) -> Result<(Vec<w::Entity>, Option<Position>), Status> {
        use std::ops::Bound::{Excluded, Included, Unbounded};
        let from = filter
            .period
            .as_ref()
            .and_then(|p| p.from_ms)
            .unwrap_or(i64::MIN);
        let until = filter.period.as_ref().and_then(|p| p.until_ms);
        let start = after
            .filter(|p| p.executed_at_ms >= from)
            .map_or(Included((from, "", "")), |p| Excluded(p.stored()));
        if until
            .is_some_and(|until| from >= until || after.is_some_and(|p| p.executed_at_ms >= until))
        {
            return Ok((Vec::new(), None));
        }
        let end = until.map_or(Unbounded, |time| Excluded((time, "", "")));
        let executions = self.read.open_table(EXECUTIONS).map_err(failure)?;
        let index = self.read.open_table(INDEX).map_err(failure)?;
        let candidate = filter.index()?;
        let rows: Box<dyn Iterator<Item = Result<Position, Status>> + '_> =
            if let Some(IndexTerm { kind, path, value }) = &candidate {
                let lo = match start {
                    Included(t) => Included((*kind as u8, path.as_slice(), value.as_slice(), t)),
                    Excluded(t) => Excluded((*kind as u8, path.as_slice(), value.as_slice(), t)),
                    Unbounded => unreachable!(),
                };
                let hi = until.map_or(
                    Included((
                        *kind as u8,
                        path.as_slice(),
                        value.as_slice(),
                        (i64::MAX, "\u{10ffff}", "\u{10ffff}"),
                    )),
                    |time| {
                        Excluded((
                            *kind as u8,
                            path.as_slice(),
                            value.as_slice(),
                            (time, "", ""),
                        ))
                    },
                );
                Box::new(index.range((lo, hi)).map_err(failure)?.map(|row| {
                    let (key, _) = row.map_err(failure)?;
                    let (_, _, _, (t, v, id)) = key.value();
                    Ok(Position {
                        executed_at_ms: t,
                        venue_id: v.into(),
                        order_id: id.into(),
                    })
                }))
            } else {
                Box::new(executions.range((start, end)).map_err(failure)?.map(|row| {
                    let (key, _) = row.map_err(failure)?;
                    let (t, v, id) = key.value();
                    Ok(Position {
                        executed_at_ms: t,
                        venue_id: v.into(),
                        order_id: id.into(),
                    })
                }))
            };
        let mut data = Vec::new();
        let mut position = after.cloned();
        let mut bytes = 0;
        for (count, row) in rows.enumerate() {
            let row = row?;
            if count == crate::READ_LIMITS.max_scan_candidates {
                return Ok((data, position));
            }
            let raw = executions
                .get(row.stored())
                .map_err(failure)?
                .ok_or_else(|| failure("index points to missing execution"))?;
            let order = w::Order::decode(raw.value()).map_err(failure)?;
            if filter.matches_order(&order)? {
                let entity = w::Entity {
                    entity: Some(w::entity::Entity::Order(order)),
                };
                if include(&entity)? {
                    if data.len() == limit {
                        return Ok((data, position));
                    }
                    let size = entity.encoded_len() + 8;
                    if bytes + size > crate::READ_LIMITS.max_read_bytes {
                        if data.is_empty() {
                            return Err(Status::resource_exhausted("order exceeds page budget"));
                        }
                        return Ok((data, position));
                    }
                    bytes += size;
                    data.push(entity);
                }
            }
            position = Some(row);
        }
        Ok((data, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_database_tuple_keys_remain_readable_and_writable() {
        let directory = tempfile::tempdir().unwrap();
        let book = w::BookKey {
            venue_id: "venue".into(),
            item_type_id: "gift".into(),
            currency_id: "GRAM".into(),
        };
        let active = w::Order {
            id: "active".into(),
            book: Some(book.clone()),
            mode: 1,
            price: "1.25".into(),
            quantity: "1".into(),
            ..Default::default()
        };
        let executed = w::Order {
            id: "old".into(),
            mode: 3,
            executed_at_ms: Some(10),
            ..active.clone()
        };
        // Write the old persisted representation directly, independently of typed key conversion.
        {
            let database = Database::create(directory.path().join("replica.redb")).unwrap();
            let tx = database.begin_write().unwrap();
            {
                let mut meta = tx.open_table(META).unwrap();
                meta.insert("source", b"legacy-source".as_slice()).unwrap();
                meta.insert("cursor", b"legacy-checkpoint".as_slice())
                    .unwrap();
                let mut current = tx.open_table(CURRENT).unwrap();
                for (kind, id, venue, entity) in [
                    (
                        1,
                        "venue",
                        "",
                        w::entity::Entity::Venue(w::Venue {
                            id: "venue".into(),
                            name: "Venue".into(),
                        }),
                    ),
                    (
                        2,
                        "GRAM",
                        "",
                        w::entity::Entity::Currency(w::Currency {
                            id: "GRAM".into(),
                            name: "Gram".into(),
                            decimals: 9,
                        }),
                    ),
                    (
                        3,
                        "gift",
                        "",
                        w::entity::Entity::ItemType(w::ItemType {
                            id: "gift".into(),
                            name: "Gift".into(),
                            quantity_step: "1".into(),
                            ..Default::default()
                        }),
                    ),
                    (
                        7,
                        "active",
                        "venue",
                        w::entity::Entity::Order(active.clone()),
                    ),
                ] {
                    current
                        .insert(
                            (kind, venue, id),
                            w::Entity {
                                entity: Some(entity),
                            }
                            .encode_to_vec()
                            .as_slice(),
                        )
                        .unwrap();
                }
                tx.open_table(EXECUTIONS)
                    .unwrap()
                    .insert((10, "venue", "old"), executed.encode_to_vec().as_slice())
                    .unwrap();
                tx.open_table(IDENTITIES)
                    .unwrap()
                    .insert(("venue", "old"), 10)
                    .unwrap();
                tx.open_table(INDEX)
                    .unwrap()
                    .insert(
                        (
                            2,
                            b"gift".as_slice(),
                            b"GRAM".as_slice(),
                            (10, "venue", "old"),
                        ),
                        (),
                    )
                    .unwrap();
                tx.open_table(JOURNAL).unwrap();
            }
            tx.commit().unwrap();
        }
        let mut writer = crate::ReplicaWriter::open(directory.path(), "legacy-source").unwrap();
        let replica = writer.reader.clone();
        let query = |mode, sort| w::ReadRequest {
            selection: Some(w::Selection {
                source: Some(w::selection::Source::Scope(w::Scope {
                    item_type_id: Some("gift".into()),
                    currency_id: Some("GRAM".into()),
                    modes: vec![mode],
                    ..Default::default()
                })),
                ..Default::default()
            }),
            sort,
            ..Default::default()
        };
        assert_eq!(
            state::order(&replica.read(query(1, 1)).unwrap().page.entities[0])
                .unwrap()
                .id,
            "active"
        );
        assert_eq!(
            state::order(&replica.read(query(3, 0)).unwrap().page.entities[0])
                .unwrap()
                .id,
            "old"
        );
        for sort in [1, 2, 3, 4, 5, 6] {
            assert_eq!(
                state::order(&replica.read(query(3, sort)).unwrap().page.entities[0])
                    .unwrap()
                    .id,
                "old",
                "legacy history index backfill, sort {sort}"
            );
        }
        writer
            .apply(w::Batch {
                cursor: "typed-checkpoint".into(),
                changes: vec![w::Change {
                    change: Some(w::change::Change::Upsert(w::Entity {
                        entity: Some(w::entity::Entity::Order(w::Order {
                            mode: 3,
                            executed_at_ms: Some(20),
                            ..active
                        })),
                    })),
                }],
            })
            .unwrap();
        assert!(replica.read(query(1, 1)).unwrap().page.entities.is_empty());
        assert_eq!(replica.read(query(3, 0)).unwrap().page.entities.len(), 2);
    }
}
