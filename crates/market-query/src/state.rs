//! Read materialization: one immutable row map with its query indexes, replaced atomically.
use crate::indexes::Indexes;
use im::OrdMap;
use market_view::*;
use std::{
    collections::BTreeSet,
    ops::Bound::{Excluded, Unbounded},
    sync::Arc,
};
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("replication cursor gap")]
    Gap,
}
#[derive(Debug, Clone)]
pub struct PreparedChanges {
    catalog: Arc<Catalog>,
    entries: Vec<(Mutation, usize)>,
}
impl PreparedChanges {
    pub fn new(
        catalog: Arc<Catalog>,
        mutations: Vec<Mutation>,
        measure: impl Fn(&Record) -> usize,
    ) -> Result<Self, ValidationError> {
        let mut keys = BTreeSet::new();
        let mut entries = Vec::new();
        for m in mutations {
            catalog.validate_mutation(&m)?;
            if !keys.insert(m.key()) {
                return Err(ValidationError("duplicate delta key"));
            }
            let weight = if let Mutation::Upsert(r) = &m {
                measure(r)
            } else {
                0
            };
            entries.push((m, weight));
        }
        Ok(Self { catalog, entries })
    }
    pub fn mutations(&self) -> impl ExactSizeIterator<Item = &Mutation> {
        self.entries.iter().map(|(m, _)| m)
    }
    pub fn weighted(&self) -> impl Iterator<Item = (&Mutation, usize)> {
        self.entries.iter().map(|(m, w)| (m, *w))
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
#[derive(Debug, Clone)]
struct Row {
    record: Record,
    bytes: usize,
}
#[derive(Debug, Clone)]
pub struct MarketState {
    catalog: Arc<Catalog>,
    rows: OrdMap<RecordKey, Arc<Row>>,
    bytes: usize,
    pub(crate) indexes: Indexes,
}
impl Default for MarketState {
    fn default() -> Self {
        Self::new(Arc::new(Catalog::default()))
    }
}
impl MarketState {
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self {
            catalog,
            rows: OrdMap::new(),
            bytes: 0,
            indexes: Indexes::default(),
        }
    }

    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn total_weight(&self) -> usize {
        self.bytes
    }
    pub fn get(&self, key: &RecordKey) -> Option<&Record> {
        self.rows.get(key).map(|r| &r.record)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&RecordKey, &Record)> {
        self.rows.iter().map(|(k, r)| (k, &r.record))
    }
    pub fn after<'a>(
        &'a self,
        key: Option<&'a RecordKey>,
    ) -> impl Iterator<Item = (&'a RecordKey, &'a Record)> {
        self.rows
            .range((key.map_or(Unbounded, Excluded), Unbounded))
            .map(|(k, r)| (k, &r.record))
    }
    pub fn with_catalog(&self, catalog: Arc<Catalog>) -> Result<Self, ValidationError> {
        for (_, r) in self.iter() {
            catalog.validate_mutation(&Mutation::Upsert(r.clone()))?
        }
        let mut n = self.clone();
        n.catalog = catalog;
        Ok(n)
    }
    pub fn apply(&mut self, mutations: &[Mutation]) -> Result<(), ApplyError> {
        let prepared = PreparedChanges::new(self.catalog.clone(), mutations.to_vec(), |r| {
            use prost::Message;
            hub_protocol::wire::Record::from(r.clone()).encoded_len()
        })?;
        self.apply_prepared(&prepared)
    }
    pub fn apply_prepared(&mut self, p: &PreparedChanges) -> Result<(), ApplyError> {
        let mut next = self.clone();
        next.catalog = p.catalog.clone();
        let mut reindex = BTreeSet::new();
        for (m, _) in &p.entries {
            let key = m.key();
            let changed = match (self.get(&key), m) {
                (Some(old), Mutation::Upsert(new)) => !Indexes::unchanged(old, new),
                _ => true,
            };
            if changed {
                reindex.insert(key.clone());
            }
            if let Some(old) = next.rows.remove(&key) {
                next.bytes -= old.bytes;
                if changed {
                    next.indexes.remove(&old.record);
                }
            }
        }
        for (m, bytes) in &p.entries {
            if let Mutation::Upsert(r) = m {
                let key = m.key();
                if reindex.contains(&key) {
                    next.indexes.insert(r)?;
                }
                next.bytes += bytes;
                next.rows.insert(
                    key,
                    Arc::new(Row {
                        record: r.clone(),
                        bytes: *bytes,
                    }),
                );
            }
        }
        *self = next;
        Ok(())
    }
    pub fn apply_prepared_delta(
        &mut self,
        cursor: &mut Cursor,
        next: &Cursor,
        p: &PreparedChanges,
    ) -> Result<(), ApplyError> {
        if cursor.generation != next.generation
            || cursor.revision.checked_add(1) != Some(next.revision)
        {
            return Err(ApplyError::Gap);
        }
        self.apply_prepared(p)?;
        *cursor = next.clone();
        Ok(())
    }
    pub fn apply_changes(
        &mut self,
        cursor: &mut Cursor,
        next: &Cursor,
        mutations: &[Mutation],
    ) -> Result<(), ApplyError> {
        let p = PreparedChanges::new(self.catalog.clone(), mutations.to_vec(), |r| {
            use prost::Message;
            hub_protocol::wire::Record::from(r.clone()).encoded_len()
        })?;
        self.apply_prepared_delta(cursor, next, &p)
    }
    pub fn apply_delta(
        &mut self,
        cursor: &mut Cursor,
        batch: &ChangeBatch,
    ) -> Result<(), ApplyError> {
        self.apply_changes(
            cursor,
            &batch.cursor,
            &batch
                .events
                .iter()
                .filter_map(MarketEvent::mutation)
                .collect::<Vec<_>>(),
        )
    }
}
