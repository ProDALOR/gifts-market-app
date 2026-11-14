use crate::{
    query::Filter,
    state::{self, State},
};
use hub_protocol::wire as w;
use prost::Message;
use redb::{
    Database, Durability, ReadTransaction, ReadableDatabase, ReadableTable, ReadableTableMetadata,
    TableDefinition,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path, sync::Arc};
use tonic::Status;

pub(crate) type IndexTerm = (u8, Vec<u8>, Vec<u8>);
pub(crate) type Leaf = (Vec<u8>, Vec<u8>);
type Identity<'a> = (i32, &'a str, &'a str);
type TimeKey<'a> = (i64, &'a str, &'a str);
type IndexKey<'a> = (u8, &'a [u8], &'a [u8], TimeKey<'a>);
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata");
const CURRENT: TableDefinition<Identity<'static>, &[u8]> = TableDefinition::new("current");
const EXECUTIONS: TableDefinition<TimeKey<'static>, &[u8]> = TableDefinition::new("executions");
const IDENTITIES: TableDefinition<(&str, &str), i64> = TableDefinition::new("identities");
const INDEX: TableDefinition<IndexKey<'static>, ()> = TableDefinition::new("index");
const JOURNAL: TableDefinition<u64, &[u8]> = TableDefinition::new("bootstrap_journal");
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Position(pub i64, pub String, pub String);
impl Position {
    fn key(&self) -> TimeKey<'_> {
        (self.0, &self.1, &self.2)
    }
}
#[derive(Clone)]
pub struct Store(Arc<Database>);
#[derive(Clone)]
pub struct Snapshot {
    pub read: Arc<ReadTransaction>,
    pub _store: Store,
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
        tx.commit().map_err(failure)?;
        Ok(Self(Arc::new(database)))
    }
    pub fn snapshot(&self) -> Result<Snapshot, Status> {
        Ok(Snapshot {
            read: Arc::new(self.0.begin_read().map_err(failure)?),
            _store: self.clone(),
        })
    }
    pub fn cursor(&self) -> Result<String, Status> {
        let snap = self.snapshot()?;
        let meta = snap.read.open_table(META).map_err(failure)?;
        String::from_utf8(value(&meta, "cursor")?).map_err(failure)
    }
    pub fn updated_at(&self) -> Result<Option<i64>, Status> {
        let snap = self.snapshot()?;
        let meta = snap.read.open_table(META).map_err(failure)?;
        let raw = value(&meta, "updated_at")?;
        if raw.is_empty() {
            Ok(None)
        } else {
            String::from_utf8(raw)
                .map_err(failure)?
                .parse()
                .map(Some)
                .map_err(failure)
        }
    }
    pub fn state(&self) -> Result<State, Status> {
        let snap = self.snapshot()?;
        let table = snap.read.open_table(CURRENT).map_err(failure)?;
        let mut state = State::default();
        for row in table.iter().map_err(failure)? {
            let (_, payload) = row.map_err(failure)?;
            state.upsert(w::Entity::decode(payload.value()).map_err(failure)?)?;
        }
        Ok(state)
    }
    pub fn apply(&self, changes: &[w::Change], cursor: Option<&str>) -> Result<(), Status> {
        let mut tx = self.0.begin_write().map_err(failure)?;
        tx.set_durability(if cursor.is_some() {
            Durability::Immediate
        } else {
            Durability::None
        })
        .map_err(failure)?;
        {
            let mut current = tx.open_table(CURRENT).map_err(failure)?;
            let mut executions = tx.open_table(EXECUTIONS).map_err(failure)?;
            let mut identities = tx.open_table(IDENTITIES).map_err(failure)?;
            let mut index = tx.open_table(INDEX).map_err(failure)?;
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
                        current
                            .remove((k.kind, k.venue_id.as_str(), k.id.as_str()))
                            .map_err(failure)?;
                    }
                    w::change::Change::Upsert(entity) => {
                        let key = state::entity_key(entity)?;
                        if let Some(order) = state::order(entity).filter(|v| v.mode == 3) {
                            let book = order
                                .book
                                .as_ref()
                                .ok_or_else(|| Status::data_loss("missing book"))?;
                            let venue = book.venue_id.as_str();
                            let id = order.id.as_str();
                            let time = order
                                .executed_at_ms
                                .ok_or_else(|| Status::data_loss("missing execution time"))?;
                            current.remove((7, venue, id)).map_err(failure)?;
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
                                    ));
                                }
                                continue;
                            }
                            executions
                                .insert((time, venue, id), order.encode_to_vec().as_slice())
                                .map_err(failure)?;
                            identities.insert((venue, id), time).map_err(failure)?;
                            for (tag, path, value) in execution_indexes(order)? {
                                index
                                    .insert(
                                        (tag, path.as_slice(), value.as_slice(), (time, venue, id)),
                                        (),
                                    )
                                    .map_err(failure)?;
                            }
                        } else {
                            current
                                .insert(
                                    (key.0, key.1.as_str(), key.2.as_str()),
                                    entity.encode_to_vec().as_slice(),
                                )
                                .map_err(failure)?;
                        }
                    }
                }
            }
            if let Some(cursor) = cursor {
                if cursor.is_empty() {
                    return Err(Status::data_loss("empty durable checkpoint"));
                }
                let mut meta = tx.open_table(META).map_err(failure)?;
                meta.insert("cursor", cursor.as_bytes()).map_err(failure)?;
                meta.insert("updated_at", crate::now().to_string().as_bytes())
                    .map_err(failure)?;
            }
        }
        tx.commit().map_err(failure)
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
    pub fn journal_at(&self, position: u64) -> Result<Option<w::Batch>, Status> {
        let snap = self.snapshot()?;
        let table = snap.read.open_table(JOURNAL).map_err(failure)?;
        table
            .get(position)
            .map_err(failure)?
            .map(|raw| w::Batch::decode(raw.value()).map_err(failure))
            .transpose()
    }
    pub fn finish(&self, cursor: &str) -> Result<(), Status> {
        self.apply(&[], Some(cursor))?;
        let tx = self.0.begin_write().map_err(failure)?;
        tx.delete_table(JOURNAL).map_err(failure)?;
        tx.open_table(JOURNAL).map_err(failure)?;
        tx.commit().map_err(failure)
    }
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
            out.insert((
                serde_json::to_vec(path).map_err(failure)?,
                value.encode_to_vec(),
            ));
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
        (0, vec![], b.venue_id.as_bytes().to_vec()),
        (1, vec![], b.item_type_id.as_bytes().to_vec()),
        (
            2,
            b.item_type_id.as_bytes().to_vec(),
            b.currency_id.as_bytes().to_vec(),
        ),
    ]
    .into();
    for (id, predicate) in &order.attributes {
        if let Some(w::attribute_predicate::Predicate::Eq(value)) = &predicate.predicate {
            let mut facts = BTreeSet::new();
            leaves(&mut vec![id.clone()], value, &mut facts)?;
            rows.extend(facts.into_iter().map(|(path, value)| (3, path, value)));
        }
    }
    Ok(rows)
}
impl Snapshot {
    /// Scan a necessary index in chronological order, checking the full predicate afterwards.
    pub fn history(
        &self,
        filter: &Filter,
        after: Option<&Position>,
        limit: usize,
    ) -> Result<(Vec<w::Entity>, Option<Position>), Status> {
        use std::ops::Bound::{Excluded, Included, Unbounded};
        let from = filter
            .period
            .as_ref()
            .and_then(|p| p.from_ms)
            .unwrap_or(i64::MIN);
        let until = filter.period.as_ref().and_then(|p| p.until_ms);
        let start = after
            .filter(|p| p.0 >= from)
            .map_or(Included((from, "", "")), |p| Excluded(p.key()));
        if until.is_some_and(|until| from >= until || after.is_some_and(|p| p.0 >= until)) {
            return Ok((Vec::new(), None));
        }
        let end = until.map_or(Unbounded, |time| Excluded((time, "", "")));
        let executions = self.read.open_table(EXECUTIONS).map_err(failure)?;
        let index = self.read.open_table(INDEX).map_err(failure)?;
        let candidate = filter.index()?;
        let rows: Box<dyn Iterator<Item = Result<Position, Status>> + '_> =
            if let Some((tag, path, value)) = &candidate {
                let lo = match start {
                    Included(t) => Included((*tag, path.as_slice(), value.as_slice(), t)),
                    Excluded(t) => Excluded((*tag, path.as_slice(), value.as_slice(), t)),
                    Unbounded => unreachable!(),
                };
                let hi = until.map_or(
                    Included((
                        *tag,
                        path.as_slice(),
                        value.as_slice(),
                        (i64::MAX, "\u{10ffff}", "\u{10ffff}"),
                    )),
                    |time| Excluded((*tag, path.as_slice(), value.as_slice(), (time, "", ""))),
                );
                Box::new(index.range((lo, hi)).map_err(failure)?.map(|row| {
                    let (key, _) = row.map_err(failure)?;
                    let (_, _, _, (t, v, id)) = key.value();
                    Ok(Position(t, v.into(), id.into()))
                }))
            } else {
                Box::new(executions.range((start, end)).map_err(failure)?.map(|row| {
                    let (key, _) = row.map_err(failure)?;
                    let (t, v, id) = key.value();
                    Ok(Position(t, v.into(), id.into()))
                }))
            };
        let mut data = Vec::new();
        let mut position = after.cloned();
        let mut bytes = 0;
        for (count, row) in rows.enumerate() {
            let row = row?;
            if data.len() == limit || count == 10_000 {
                return Ok((data, position));
            }
            let raw = executions
                .get(row.key())
                .map_err(failure)?
                .ok_or_else(|| failure("index points to missing execution"))?;
            let order = w::Order::decode(raw.value()).map_err(failure)?;
            if filter.matches_order(&order)? {
                let entity = w::Entity {
                    entity: Some(w::entity::Entity::Order(order)),
                };
                let size = entity.encoded_len() + 8;
                if bytes + size > 512 * 1024 {
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
