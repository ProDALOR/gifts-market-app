use crate::{
    query::Filter,
    state::{self, State},
    types::{IndexKind, IndexTerm, Key, Leaf, OrderMode, Sort},
};
use market_hub_protocol::wire as w;
use prost::Message;
use redb::{
    Database, Durability, ReadTransaction, ReadableDatabase, ReadableTable, ReadableTableMetadata,
    TableDefinition, TableError, TableHandle, WriteTransaction,
};
use std::time::{Duration, Instant};
use std::{collections::BTreeSet, path::Path, sync::Arc};
use tonic::Status;

pub use crate::types::Position;
type Identity<'a> = (i32, &'a str, &'a str);
type TimeKey<'a> = (i64, &'a str, &'a str);
type IndexKey<'a> = (u8, &'a [u8], &'a [u8], TimeKey<'a>);
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata");
const CURRENT: TableDefinition<Identity<'static>, &[u8]> = TableDefinition::new("current");
const EXECUTIONS: TableDefinition<TimeKey<'static>, &[u8]> = TableDefinition::new("executions");
pub(super) const IDENTITIES: TableDefinition<(&str, &str), i64> =
    TableDefinition::new("identities");
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
    pub(super) pending: super::pending_history::Pending,
}
pub(crate) struct Applied {
    pub updated_at: Option<i64>,
    pub data_changed: bool,
    pub history_changed: bool,
    pub timings: WriteTimings,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct WriteTimings {
    pub wait: Duration,
    pub write: Duration,
    pub commit: Duration,
    pub history_payload: Duration,
    pub history_indexes: Duration,
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
// Validate one committed schema before exposing a Store. Only a truly empty
// database can be initialized; saved checkpoints must not adopt missing data.
fn initialized(database: &Database, source: &str) -> Result<bool, Status> {
    let read = database.begin_read().map_err(failure)?;
    let meta = match read.open_table(META) {
        Ok(meta) => meta,
        Err(TableError::TableDoesNotExist(_)) => {
            if read.list_tables().map_err(failure)?.next().is_some() {
                return Err(Status::data_loss(
                    "replica database is missing its service identity",
                ));
            }
            return Ok(false);
        }
        Err(error) => return Err(failure(error)),
    };
    let stored = meta.get("source").map_err(failure)?;
    let stored = stored
        .as_ref()
        .map(|value| value.value())
        .unwrap_or_default();
    if stored.is_empty() {
        return Err(Status::data_loss(
            "replica database is missing its service identity",
        ));
    }
    if stored != source.as_bytes() {
        return Err(Status::failed_precondition(
            "data directory belongs to another hub or service identity",
        ));
    }
    macro_rules! exists {
        ($table:expr) => {
            match read.open_table($table) {
                Ok(_) => true,
                Err(TableError::TableDoesNotExist(_)) => false,
                Err(error) => return Err(failure(error)),
            }
        };
    }
    for (present, name) in [
        (exists!(CURRENT), CURRENT.name()),
        (exists!(EXECUTIONS), EXECUTIONS.name()),
        (exists!(IDENTITIES), IDENTITIES.name()),
        (exists!(INDEX), INDEX.name()),
    ] {
        if !present {
            return Err(Status::data_loss(format!(
                "replica database is missing required table {name}"
            )));
        }
    }
    // Missing staging journals can be created; ranked history can be rebuilt.
    // Validate every existing table's type even when the rebuild marker is absent.
    let journal = exists!(JOURNAL);
    let ranked = exists!(SORTED);
    let ready = meta
        .get("execution_ranks_v1")
        .map_err(failure)?
        .is_some_and(|v| !v.value().is_empty());
    Ok(journal && ranked && ready)
}
impl Store {
    pub fn open(path: &Path, source: &str) -> Result<Self, Status> {
        Self::open_checked(path, source, &|| Ok(()))
    }
    pub(crate) fn open_checked(
        path: &Path,
        source: &str,
        check: &impl Fn() -> Result<(), Status>,
    ) -> Result<Self, Status> {
        check()?;
        if source.is_empty() {
            return Err(Status::invalid_argument("empty replica service identity"));
        }
        std::fs::create_dir_all(
            path.parent()
                .ok_or_else(|| failure("missing data directory"))?,
        )
        .map_err(failure)?;
        let started = Instant::now();
        tracing::info!("opening replica database");
        let database = Database::create(path).map_err(failure)?;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            "replica database opened"
        );
        check()?;
        if initialized(&database, source)? {
            tracing::info!(
                elapsed_ms = started.elapsed().as_millis(),
                "replica storage validated"
            );
            return Ok(Self(
                Arc::new(database),
                #[cfg(test)]
                Arc::default(),
            ));
        }
        check()?;
        let tx = database.begin_write().map_err(failure)?;
        // No database handle is shared until open returns, so the read-only
        // identity/schema inspection cannot race another application writer.
        tx.open_table(META)
            .map_err(failure)?
            .insert("source", source.as_bytes())
            .map_err(failure)?;
        tx.open_table(CURRENT).map_err(failure)?;
        tx.open_table(EXECUTIONS).map_err(failure)?;
        tx.open_table(IDENTITIES).map_err(failure)?;
        tx.open_table(INDEX).map_err(failure)?;
        tx.open_table(JOURNAL).map_err(failure)?;
        {
            let mut meta = tx.open_table(META).map_err(failure)?;
            let ranks_present = tx
                .list_tables()
                .map_err(failure)?
                .any(|table| table.name() == SORTED.name());
            if !ranks_present || value(&meta, "execution_ranks_v1")?.is_empty() {
                // Replace the entire derived index in this transaction. A failed
                // rebuild restores the old table and marker together on rollback.
                tx.delete_table(SORTED).map_err(failure)?;
                let mut ranked = tx.open_table(SORTED).map_err(failure)?;
                let executions = tx.open_table(EXECUTIONS).map_err(failure)?;
                let rows = executions.len().map_err(failure)?;
                let indexing = Instant::now();
                let mut progress = Instant::now();
                tracing::info!(rows, "building missing execution ranks");
                for (index, row) in executions.iter().map_err(failure)?.enumerate() {
                    check()?;
                    let (stored, raw) = row.map_err(failure)?;
                    let order = w::Order::decode(raw.value()).map_err(failure)?;
                    let identity = order
                        .book
                        .as_ref()
                        .zip(order.executed_at_ms)
                        .map(|(book, time)| (time, book.venue_id.as_str(), order.id.as_str()));
                    if order.mode != w::OrderMode::Executed as i32
                        || identity != Some(stored.value())
                    {
                        return Err(Status::data_loss(
                            "stored execution identity does not match payload",
                        ));
                    }
                    insert_ranks(&mut ranked, &order)?;
                    let indexed = index + 1;
                    if indexed.is_multiple_of(10_000)
                        && progress.elapsed() >= Duration::from_secs(5)
                    {
                        tracing::info!(
                            indexed,
                            rows,
                            elapsed_ms = indexing.elapsed().as_millis(),
                            "execution rank rebuild progress"
                        );
                        progress = Instant::now();
                    }
                }
                tracing::info!(
                    rows,
                    elapsed_ms = indexing.elapsed().as_millis(),
                    "execution ranks prepared"
                );
                meta.insert("execution_ranks_v1", b"1".as_slice())
                    .map_err(failure)?;
            }
        }
        check()?;
        tx.commit().map_err(failure)?;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            "replica storage initialized"
        );
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
            pending: Default::default(),
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
        self.apply_with_index_order(changes, cursor, cursor.is_none())
    }
    pub(crate) fn apply_with_index_order<'a>(
        &self,
        changes: impl IntoIterator<Item = &'a w::Change>,
        cursor: Option<&str>,
        bulk: bool,
    ) -> Result<Applied, ApplyError> {
        let waiting = Instant::now();
        let mut tx = self.0.begin_write().map_err(failure)?;
        let wait = waiting.elapsed();
        let writing = Instant::now();
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
            timings: WriteTimings {
                wait,
                ..Default::default()
            },
        };
        let mut changes = changes.into_iter().peekable();
        if changes.peek().is_some() {
            let mut current = tx.open_table(CURRENT).map_err(failure)?;
            let mut executions = tx.open_table(EXECUTIONS).map_err(failure)?;
            let mut identities = tx.open_table(IDENTITIES).map_err(failure)?;
            let mut index = tx.open_table(INDEX).map_err(failure)?;
            let mut ranked = tx.open_table(SORTED).map_err(failure)?;
            let mut encoded = Vec::new();
            let mut pending = bulk.then(HistoryIndexes::default);
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
                            let payload_started = Instant::now();
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
                                applied.timings.history_payload += payload_started.elapsed();
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
                            applied.timings.history_payload += payload_started.elapsed();
                            let indexes_started = Instant::now();
                            if let Some(pending) = &mut pending {
                                pending.add(order)?;
                                if pending.bytes >= crate::limits::MAX_HISTORY_INDEX_BYTES {
                                    pending.flush(&mut ranked, &mut index)?;
                                }
                            } else {
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
                            }
                            applied.timings.history_indexes += indexes_started.elapsed();
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
            if let Some(pending) = &mut pending {
                let started = Instant::now();
                pending.flush(&mut ranked, &mut index)?;
                applied.timings.history_indexes += started.elapsed();
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
        applied.timings.write = writing.elapsed();
        let committing = Instant::now();
        tx.commit().map_err(|error| ApplyError {
            status: failure(error),
            commit_attempted: true,
        })?;
        applied.timings.commit = committing.elapsed();
        #[cfg(test)]
        if fault == 2 {
            return Err(ApplyError {
                status: Status::unavailable("test uncertain commit"),
                commit_attempted: true,
            });
        }
        Ok(applied)
    }
    pub fn journal(&self, batches: &[w::Batch], buffer: &mut Vec<u8>) -> Result<usize, Status> {
        if batches.is_empty() {
            return Ok(0);
        }
        let mut tx = self.0.begin_write().map_err(failure)?;
        tx.set_durability(Durability::None).map_err(failure)?;
        let mut bytes = 0;
        {
            let mut table = tx.open_table(JOURNAL).map_err(failure)?;
            let next = table.len().map_err(failure)?;
            for (offset, batch) in batches.iter().enumerate() {
                buffer.clear();
                buffer.reserve(batch.encoded_len());
                batch.encode(&mut *buffer).map_err(failure)?;
                bytes += buffer.len();
                table
                    .insert(next + offset as u64, buffer.as_slice())
                    .map_err(failure)?;
            }
        }
        tx.commit().map_err(failure)?;
        Ok(bytes)
    }
    #[cfg(test)]
    pub(crate) fn hold_write_for_test(&self) -> WriteTransaction {
        self.0.begin_write().unwrap()
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
const HISTORY_SORTS: [Sort; 3] = [Sort::PriceAsc, Sort::TimeAsc, Sort::NumberAsc];
type OwnedRank<'a> = (u8, &'a str, &'a str, Vec<u8>, TimeKey<'a>);
type OwnedFilter<'a> = (u8, Vec<u8>, Vec<u8>, TimeKey<'a>);
#[derive(Default)]
struct HistoryIndexes<'a> {
    ranks: Vec<OwnedRank<'a>>,
    filters: Vec<OwnedFilter<'a>>,
    bytes: usize,
}
impl<'a> HistoryIndexes<'a> {
    fn add(&mut self, order: &'a w::Order) -> Result<(), Status> {
        let book = order
            .book
            .as_ref()
            .ok_or_else(|| Status::data_loss("missing book"))?;
        let time = order
            .executed_at_ms
            .ok_or_else(|| Status::data_loss("missing execution time"))?;
        let key = (time, book.venue_id.as_str(), order.id.as_str());
        for sort in HISTORY_SORTS {
            let rank = crate::ranking::ordered_decimal(sort.rank(order, true)?);
            self.bytes += std::mem::size_of::<OwnedRank<'a>>() + rank.len();
            self.ranks.push((
                sort.criterion(),
                book.item_type_id.as_str(),
                book.currency_id.as_str(),
                rank,
                key,
            ));
        }
        for IndexTerm { kind, path, value } in execution_indexes(order)? {
            self.bytes += std::mem::size_of::<OwnedFilter<'a>>() + path.len() + value.len();
            self.filters.push((kind as u8, path, value, key));
        }
        Ok(())
    }
    fn flush(
        &mut self,
        ranked: &mut redb::Table<'_, RankedExecution<'static>, ()>,
        index: &mut redb::Table<'_, IndexKey<'static>, ()>,
    ) -> Result<(), Status> {
        // Staging is private: reorder derived writes within the same transaction,
        // while payload/identity validation remains in original delivery order.
        self.ranks.sort_unstable();
        self.filters.sort_unstable();
        for (kind, item_type, currency, rank, key) in self.ranks.drain(..) {
            ranked
                .insert((kind, item_type, currency, rank.as_slice(), key), ())
                .map_err(failure)?;
        }
        for (kind, path, value, key) in self.filters.drain(..) {
            index
                .insert((kind, path.as_slice(), value.as_slice(), key), ())
                .map_err(failure)?;
        }
        self.bytes = 0;
        Ok(())
    }
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
    for sort in HISTORY_SORTS {
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
        self.state_checked(&|| Ok(()))
    }
    pub(crate) fn state_checked(
        &self,
        check: &impl Fn() -> Result<(), Status>,
    ) -> Result<State, Status> {
        check()?;
        let table = self.read.open_table(CURRENT).map_err(failure)?;
        let rows = table.len().map_err(failure)?;
        let started = Instant::now();
        let mut progress = Instant::now();
        tracing::info!(rows, "restoring replica state and order indexes");
        let mut state = State::default();
        for (index, row) in table.iter().map_err(failure)?.enumerate() {
            check()?;
            let (stored, payload) = row.map_err(failure)?;
            let entity = w::Entity::decode(payload.value()).map_err(failure)?;
            let key = state::entity_key(&entity)?;
            if key.stored() != stored.value() {
                return Err(Status::data_loss(
                    "stored entity identity does not match payload",
                ));
            }
            state.insert_restored(key, entity)?;
            let restored = index + 1;
            if restored.is_multiple_of(10_000) && progress.elapsed() >= Duration::from_secs(5) {
                tracing::info!(
                    restored,
                    rows,
                    elapsed_ms = started.elapsed().as_millis(),
                    "replica state restore progress"
                );
                progress = Instant::now();
            }
        }
        tracing::info!(
            rows,
            elapsed_ms = started.elapsed().as_millis(),
            "replica state restored"
        );
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
        Ok(self.order_from(&table, position)?.map(|order| w::Entity {
            entity: Some(w::entity::Entity::Order(order)),
        }))
    }
    fn order_from(
        &self,
        table: &impl ReadableTable<TimeKey<'static>, &'static [u8]>,
        position: &Position,
    ) -> Result<Option<w::Order>, Status> {
        if let Some(order) = self.pending.order(position) {
            return Ok(Some(order.as_ref().clone()));
        }
        table
            .get(position.stored())
            .map_err(failure)?
            .map(|raw| {
                let order = w::Order::decode(raw.value()).map_err(failure)?;
                Ok(order)
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
        let mut scanned = 0;
        self.ranked_history_scoped(filter, after, end, limit, include, &mut scanned)
    }
    fn ranked_history_scoped(
        &self,
        filter: &Filter,
        after: Option<(String, Position)>,
        end: Option<(String, Position)>,
        limit: usize,
        include: &dyn Fn(&w::Entity) -> Result<bool, Status>,
        scanned: &mut usize,
    ) -> Result<RankedHistoryPage, Status> {
        if filter
            .scope
            .as_ref()
            .is_some_and(|s| s.item_type_id.is_none())
        {
            return self.ranked_history_collections(filter, after, end, limit, include, scanned);
        }
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
        let rows = rows.map(|row| {
            let (key, _) = row.map_err(failure)?;
            let (_, _, _, rank, (time, venue, id)) = key.value();
            Ok((
                <[u8; 58]>::try_from(rank).map_err(failure)?,
                Position {
                    executed_at_ms: time,
                    venue_id: venue.into(),
                    order_id: id.into(),
                },
            ))
        });
        let pending = self.pending.ranked(filter, after.as_ref(), end.as_ref())?;
        let rows = super::pending_history::merge(rows, pending, filter.sort.descending());
        let mut data = Vec::new();
        let mut bytes = 0;
        let mut position = after;
        for row in rows {
            if *scanned >= crate::READ_LIMITS.max_scan_candidates {
                return Ok((data, position));
            }
            *scanned += 1;
            let (_, row) = row?;
            let order = self
                .order_from(&executions, &row)?
                .ok_or_else(|| failure("missing indexed execution"))?;
            let rank = filter.sort.rank(&order, true)?.to_string();
            let matches = filter.matches_order(&order)?;
            let entity = w::Entity {
                entity: Some(w::entity::Entity::Order(order)),
            };
            if matches && include(&entity)? {
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
            position = Some((rank, row));
        }
        Ok((data, None))
    }
    /// Merge one bounded head per collection; do not allocate a full page for every book.
    fn ranked_history_collections(
        &self,
        filter: &Filter,
        after: Option<(String, Position)>,
        end: Option<(String, Position)>,
        limit: usize,
        include: &dyn Fn(&w::Entity) -> Result<bool, Status>,
        scanned: &mut usize,
    ) -> Result<RankedHistoryPage, Status> {
        use std::collections::BTreeMap;
        let scope = filter.scope.as_ref().unwrap();
        let collections = scope
            .collection_attributes
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing collection scopes"))?;
        let mut heads = BTreeMap::new();
        let mut retained = 0usize;
        for (id, attributes) in collections.iter() {
            let mut scoped = filter.clone();
            let target = scoped.scope.as_mut().unwrap();
            target.item_type_id = Some(id.clone());
            target.attributes = attributes.clone();
            target.collection_attributes = None;
            let (mut rows, next) = self.ranked_history_scoped(
                &scoped,
                after.clone(),
                end.clone(),
                1,
                include,
                scanned,
            )?;
            if let Some(entity) = rows.pop() {
                retained += entity.encoded_len() + 8;
                if retained > crate::READ_LIMITS.max_read_bytes {
                    return Err(Status::resource_exhausted(
                        "history merge exceeds byte budget",
                    ));
                }
                let order = state::order(&entity).ok_or_else(|| failure("expected execution"))?;
                let point = Position {
                    executed_at_ms: order
                        .executed_at_ms
                        .ok_or_else(|| failure("missing execution time"))?,
                    venue_id: order
                        .book
                        .as_ref()
                        .ok_or_else(|| failure("missing book"))?
                        .venue_id
                        .clone(),
                    order_id: order.id.clone(),
                };
                heads.insert((filter.sort.rank(order, true)?, point), (entity, scoped));
            } else if next.is_some() {
                return Err(Status::resource_exhausted(
                    "history merge exceeds scan budget",
                ));
            }
        }
        let mut rows = Vec::new();
        let mut bytes = 0;
        while let Some(((rank, point), (entity, scoped))) = if filter.sort.descending() {
            heads.pop_last()
        } else {
            heads.pop_first()
        } {
            let size = entity.encoded_len() + 8;
            if rows.len() >= limit || bytes + size > crate::READ_LIMITS.max_read_bytes {
                // A continuation is only safe after an emitted global ordering position.
                return Err(Status::resource_exhausted(
                    "history merge exceeds page budget",
                ));
            }
            bytes += size;
            retained -= size;
            rows.push(entity);
            let cursor = Some((rank.to_string(), point.clone()));
            let (mut next, continuation) = self.ranked_history_scoped(
                &scoped,
                cursor.clone(),
                end.clone(),
                1,
                include,
                scanned,
            )?;
            if let Some(entity) = next.pop() {
                retained += entity.encoded_len() + 8;
                if retained + bytes > crate::READ_LIMITS.max_read_bytes {
                    return Err(Status::resource_exhausted(
                        "history merge exceeds byte budget",
                    ));
                }
                let order = state::order(&entity).unwrap();
                let point = Position {
                    executed_at_ms: order.executed_at_ms.unwrap(),
                    venue_id: order.book.as_ref().unwrap().venue_id.clone(),
                    order_id: order.id.clone(),
                };
                heads.insert((filter.sort.rank(order, true)?, point), (entity, scoped));
            } else if continuation.is_some() {
                return Err(Status::resource_exhausted(
                    "history merge exceeds scan budget",
                ));
            }
            if rows.len() == limit {
                return Ok((
                    rows,
                    (!heads.is_empty()).then_some((rank.to_string(), point)),
                ));
            }
        }
        Ok((rows, None))
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
        let pending = self.pending.chronological(after, from, until);
        let rows = super::pending_history::merge(rows, pending, false);
        let mut data = Vec::new();
        let mut position = after.cloned();
        let mut bytes = 0;
        for (count, row) in rows.enumerate() {
            let row = row?;
            if count == crate::READ_LIMITS.max_scan_candidates {
                return Ok((data, position));
            }
            let order = self
                .order_from(&executions, &row)?
                .ok_or_else(|| failure("index points to missing execution"))?;
            let matches = filter.matches_order(&order)?;
            let entity = w::Entity {
                entity: Some(w::entity::Entity::Order(order)),
            };
            if matches && include(&entity)? {
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
            position = Some(row);
        }
        Ok((data, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_staging_indexes_match_incremental_indexes_including_duplicate_executions() {
        let mut rows = crate::benchmarks::fixture(1000);
        for (i, row) in rows.iter_mut().enumerate() {
            if let Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::Order(order)),
            })) = &mut row.change
                && i % 2 == 0
            {
                order.mode = w::OrderMode::Executed as i32;
                order.executed_at_ms = Some(i as i64 + 1);
            }
        }
        let duplicate = rows.iter().find(|row| matches!(&row.change, Some(w::change::Change::Upsert(w::Entity { entity: Some(w::entity::Entity::Order(order)) })) if order.mode == w::OrderMode::Executed as i32)).unwrap().clone();
        rows.push(duplicate);
        let mut expected = None;
        for bulk in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let store = Store::open(&directory.path().join("replica.redb"), "test").unwrap();
            store.apply_with_index_order(&rows, None, bulk).unwrap();
            let snapshot = store.snapshot().unwrap();
            let filters = snapshot
                .read
                .open_table(INDEX)
                .unwrap()
                .iter()
                .unwrap()
                .map(|row| {
                    let (key, _) = row.unwrap();
                    let (kind, path, value, (time, venue, id)) = key.value();
                    (
                        kind,
                        path.to_vec(),
                        value.to_vec(),
                        time,
                        venue.to_owned(),
                        id.to_owned(),
                    )
                })
                .collect::<Vec<_>>();
            let ranks = snapshot
                .read
                .open_table(SORTED)
                .unwrap()
                .iter()
                .unwrap()
                .map(|row| {
                    let (key, _) = row.unwrap();
                    let (kind, item, currency, value, (time, venue, id)) = key.value();
                    (
                        kind,
                        item.to_owned(),
                        currency.to_owned(),
                        value.to_vec(),
                        time,
                        venue.to_owned(),
                        id.to_owned(),
                    )
                })
                .collect::<Vec<_>>();
            let result = (filters, ranks);
            if let Some(expected) = &expected {
                assert_eq!(&result, expected);
            } else {
                expected = Some(result);
            }
        }
    }

    #[test]
    #[ignore = "manual sorted history indexing benchmark; use --test-threads=1"]
    fn history_index_batching_benchmark() {
        let mut rows = crate::benchmarks::fixture(120_000);
        for (i, row) in rows.iter_mut().enumerate() {
            if let Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::Order(order)),
            })) = &mut row.change
            {
                order.mode = w::OrderMode::Executed as i32;
                order.executed_at_ms = Some(i as i64 + 1);
                for (name, options) in [("tg.backdrop", 64), ("tg.pattern", 128)] {
                    order.attributes.insert(
                        name.into(),
                        w::AttributePredicate {
                            predicate: Some(w::attribute_predicate::Predicate::Eq(
                                w::AttributeValue {
                                    value: Some(w::attribute_value::Value::OptionId(format!(
                                        "option{}",
                                        i % options
                                    ))),
                                },
                            )),
                        },
                    );
                }
            }
        }
        let mut expected = None;
        for bulk in [false, true] {
            let mut samples = Vec::new();
            for _ in 0..3 {
                let directory = tempfile::tempdir().unwrap();
                let store =
                    Store::open(&directory.path().join("history.redb"), "benchmark").unwrap();
                let started = Instant::now();
                let mut indexes = Duration::ZERO;
                let mut payload = Duration::ZERO;
                for batch in rows.chunks(4000) {
                    let applied = store.apply_with_index_order(batch, None, bulk).unwrap();
                    indexes += applied.timings.history_indexes;
                    payload += applied.timings.history_payload;
                }
                samples.push(started.elapsed());
                let snapshot = store.snapshot().unwrap();
                let counts = (
                    snapshot.read.open_table(EXECUTIONS).unwrap().len().unwrap(),
                    snapshot.read.open_table(INDEX).unwrap().len().unwrap(),
                    snapshot.read.open_table(SORTED).unwrap().len().unwrap(),
                );
                assert_eq!(counts.0, 120_000);
                assert_eq!(counts.2, 360_000);
                assert_eq!(*expected.get_or_insert(counts), counts);
                println!(
                    "history bulk={bulk}; indexes={indexes:?}; payload={payload:?}; counts={counts:?}"
                );
            }
            samples.sort();
            println!(
                "history bulk={bulk}; 120,000 executions; median={:?}; samples={samples:?}",
                samples[1]
            );
        }
    }

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
                state::order(
                    &replica
                        .read_with_options(
                            query(3, 0),
                            crate::ReadOptions {
                                sort: Some(crate::Sort::try_from(sort).unwrap()),
                                ..Default::default()
                            }
                        )
                        .unwrap()
                        .page
                        .entities[0]
                )
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

#[cfg(test)]
#[path = "startup_benchmark.rs"]
mod startup_benchmark;

#[cfg(test)]
#[path = "startup_tests.rs"]
mod startup_tests;
