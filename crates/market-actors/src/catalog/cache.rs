//! Independently cached projections, assembled from one pinned replica view.
use super::{
    projection,
    version::{self, CatalogSnapshot},
};
use market_gifts::{
    CollectionId, Currency, GiftBook, GiftCatalog, GiftCollection, GiftCollectionCatalog, Venue,
};
use market_hub_protocol::wire::EntityKind;
use market_replica::{DirectoryRevision, MetadataRevision, ReplicaReader};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};
use tonic::Status;

type Cell<T> = Arc<OnceLock<Result<T, Status>>>;
struct Cached<T, K> {
    revision: Option<K>,
    cell: Cell<T>,
}
impl<T, K> Default for Cached<T, K> {
    fn default() -> Self {
        Self {
            revision: None,
            cell: Default::default(),
        }
    }
}
impl<T, K: PartialEq> Cached<T, K> {
    fn at(&mut self, revision: K) -> Cell<T> {
        if self.revision.as_ref() != Some(&revision) {
            self.revision = Some(revision);
            self.cell = Default::default();
        }
        self.cell.clone()
    }
}
struct ChoiceEntry {
    id: CollectionId,
    checked: MetadataRevision,
    cell: Cell<projection::ChoiceProjection>,
}
#[derive(Default)]
struct Index {
    collections: Cached<Arc<Vec<GiftCollection>>, MetadataRevision>,
    venues: Cached<Arc<Vec<Venue>>, MetadataRevision>,
    currencies: Cached<Arc<Vec<Currency>>, MetadataRevision>,
    books: Cached<Arc<Vec<GiftBook>>, (MetadataRevision, DirectoryRevision)>,
    choices: VecDeque<ChoiceEntry>,
    catalog_revision: Cached<String, (MetadataRevision, DirectoryRevision)>,
}
#[derive(Default)]
pub(crate) struct ProjectionCache(Mutex<Index>);
impl ProjectionCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, Index> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn cached<T: Clone, K: PartialEq>(
        &self,
        select: impl Fn(&mut Index) -> &mut Cached<T, K>,
        revision: K,
        build: impl FnOnce() -> Result<T, Status>,
    ) -> Result<T, Status> {
        let cell = select(&mut self.lock()).at(revision);
        let result = cell.get_or_init(build).clone();
        if result.is_err() {
            let mut index = self.lock();
            let entry = select(&mut index);
            if Arc::ptr_eq(&entry.cell, &cell) {
                // Retry failed projections without clearing an intervening newer revision.
                entry.cell = Default::default();
            }
        }
        result
    }
    pub fn bootstrap(&self, reader: ReplicaReader) -> Result<GiftCatalog, Status> {
        let reader = reader.snapshot()?;
        let gift_metadata =
            reader.metadata_revision(&[EntityKind::ItemType, EntityKind::AttributeOption])?;
        let collections = self.cached(
            |i| &mut i.collections,
            gift_metadata,
            || projection::collections(&reader),
        )?;
        let venues = self.cached(
            |i| &mut i.venues,
            reader.metadata_revision(&[EntityKind::Venue])?,
            || projection::venues(&reader),
        )?;
        let currencies = self.cached(
            |i| &mut i.currencies,
            reader.metadata_revision(&[EntityKind::Currency])?,
            || projection::currencies(&reader),
        )?;
        let books = self.cached(
            |i| &mut i.books,
            (
                reader.metadata_revision(&[EntityKind::ItemType])?,
                reader.directory_revision()?,
            ),
            || projection::books(&reader, &collections),
        )?;
        Ok(GiftCatalog {
            collections,
            venues,
            currencies,
            books,
        })
    }
    fn collection_cell(
        &self,
        reader: ReplicaReader,
        id: CollectionId,
    ) -> Result<Cell<projection::ChoiceProjection>, Status> {
        let reader = reader.snapshot()?;
        let revision =
            reader.metadata_revision(&[EntityKind::ItemType, EntityKind::AttributeOption])?;
        let previous = self
            .lock()
            .choices
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| (entry.checked, entry.cell.clone()));
        // Revalidate selected dependencies only after gift metadata changes. No catalog scans
        // or dependency walks are needed for normal hits, and no work runs under the index lock.
        let candidate = if let Some((checked, cell)) = previous {
            let valid = checked == revision
                || match cell.get() {
                    Some(Ok(projection)) => reader.metadata_matches(&projection.dependencies)?,
                    _ => false,
                };
            if valid { cell } else { Default::default() }
        } else {
            Default::default()
        };
        let cell = {
            let mut index = self.lock();
            let current = index
                .choices
                .iter()
                .position(|entry| entry.id == id)
                .and_then(|pos| index.choices.remove(pos));
            // A concurrent request may already have installed a cell for this exact view.
            let cell = current
                .filter(|entry| entry.checked == revision)
                .map(|entry| entry.cell)
                .unwrap_or(candidate);
            index.choices.push_back(ChoiceEntry {
                id: id.clone(),
                checked: revision,
                cell: cell.clone(),
            });
            while index.choices.len() > 8 {
                index.choices.pop_front();
            }
            cell
        };
        let result = cell.get_or_init(|| projection::choices(&reader, id));
        {
            let mut index = self.lock();
            // An oversized response remains usable without evicting the other collections.
            // The weight includes missing allowed IDs because they retain dependencies too.
            if result
                .as_ref()
                .map_or(true, |projection| projection.weight() > 10_000)
            {
                index
                    .choices
                    .retain(|entry| !Arc::ptr_eq(&entry.cell, &cell));
            }
            let size = |entry: &ChoiceEntry| {
                entry
                    .cell
                    .get()
                    .and_then(|value| value.as_ref().ok())
                    .map_or(0, projection::ChoiceProjection::weight)
            };
            while index.choices.iter().map(size).sum::<usize>() > 10_000 {
                index.choices.pop_front();
            }
        }
        result.as_ref().map_err(Clone::clone)?;
        Ok(cell)
    }
    #[cfg(test)]
    pub fn collection(
        &self,
        reader: ReplicaReader,
        id: CollectionId,
    ) -> Result<GiftCollectionCatalog, Status> {
        Ok(self.collection_snapshot(reader, id)?.data)
    }
    pub fn collection_snapshot(
        &self,
        reader: ReplicaReader,
        id: CollectionId,
    ) -> Result<CatalogSnapshot<GiftCollectionCatalog>, Status> {
        let cell = self.collection_cell(reader, id)?;
        let projection = cell
            .get()
            .expect("initialized collection")
            .as_ref()
            .map_err(Clone::clone)?;
        let revision = projection
            .revision
            .get_or_init(|| version::revision(&projection.catalog))
            .clone()?;
        Ok(CatalogSnapshot {
            data: projection.catalog.clone(),
            revision,
        })
    }
    pub fn bootstrap_snapshot(
        &self,
        reader: ReplicaReader,
    ) -> Result<CatalogSnapshot<GiftCatalog>, Status> {
        let reader = reader.snapshot()?;
        let data = self.bootstrap(reader.clone())?;
        let key = (
            reader.metadata_revision(&[
                EntityKind::ItemType,
                EntityKind::AttributeOption,
                EntityKind::Venue,
                EntityKind::Currency,
            ])?,
            reader.directory_revision()?,
        );
        let revision = self.cached(
            |i| &mut i.catalog_revision,
            key,
            || version::revision(&data),
        )?;
        Ok(CatalogSnapshot { data, revision })
    }
    pub fn versions(
        &self,
        reader: ReplicaReader,
        ids: Vec<CollectionId>,
    ) -> Result<market_gifts::CatalogVersions, Status> {
        let reader = reader.snapshot()?;
        let directory = self.bootstrap_snapshot(reader.clone())?.revision;
        let mut collections = std::collections::BTreeMap::new();
        for id in ids {
            let revision = match self.collection_snapshot(reader.clone(), id.clone()) {
                Ok(snapshot) => Some(snapshot.revision),
                Err(error) if error.code() == tonic::Code::NotFound => None,
                Err(error) => return Err(error),
            };
            collections.insert(id, revision);
        }
        Ok(market_gifts::CatalogVersions {
            directory,
            collections,
        })
    }
}
#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
