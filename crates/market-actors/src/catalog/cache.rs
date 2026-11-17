//! Revision-keyed projections. Immutable arrays are shared on hits; only freshness changes.
use super::gifts;
use market_gifts::{CollectionId, GiftCatalog, GiftCollectionCatalog};
use market_replica::{CatalogRevision, ReplicaReader};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};
use tonic::Status;
type Cell<T> = Arc<OnceLock<Result<T, Status>>>;
#[derive(Default)]
struct Index {
    revision: Option<CatalogRevision>,
    bootstrap: Cell<GiftCatalog>,
    collections: VecDeque<(CollectionId, Cell<GiftCollectionCatalog>)>,
}
#[derive(Default)]
pub(crate) struct ProjectionCache(Mutex<Index>);
impl ProjectionCache {
    fn index(&self, revision: CatalogRevision) -> std::sync::MutexGuard<'_, Index> {
        let mut index = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if index.revision != Some(revision) {
            *index = Index {
                revision: Some(revision),
                ..Default::default()
            };
        }
        index
    }
    pub fn bootstrap(&self, reader: ReplicaReader) -> Result<GiftCatalog, Status> {
        let reader = reader.snapshot()?;
        let revision = reader.catalog_revision()?;
        let cell = self.index(revision).bootstrap.clone();
        let result = cell
            .get_or_init(|| gifts::bootstrap(reader.clone()))
            .clone();
        if result.is_err() {
            let mut index = self.0.lock().unwrap();
            if Arc::ptr_eq(&index.bootstrap, &cell) {
                index.bootstrap = Default::default();
            }
        }
        let mut result = result?;
        result.cursor = reader.cursor()?;
        result.sync =
            crate::protocol::sync(reader.status()).map_err(|e| Status::data_loss(e.to_string()))?;
        Ok(result)
    }
    pub fn collection(
        &self,
        reader: ReplicaReader,
        id: CollectionId,
    ) -> Result<GiftCollectionCatalog, Status> {
        let reader = reader.snapshot()?;
        let revision = reader.catalog_revision()?;
        let cell = {
            let mut index = self.index(revision);
            let cell = index
                .collections
                .iter()
                .position(|(key, _)| key == &id)
                .and_then(|pos| index.collections.remove(pos))
                .map(|(_, cell)| cell)
                .unwrap_or_default();
            index.collections.push_back((id.clone(), cell.clone()));
            while index.collections.len() > 8 {
                index.collections.pop_front();
            }
            cell
        };
        let result = cell
            .get_or_init(|| gifts::details(reader.clone(), id))
            .clone();
        {
            let mut index = self.0.lock().unwrap();
            if result.is_err() {
                index
                    .collections
                    .retain(|(_, entry)| !Arc::ptr_eq(entry, &cell));
            }
            let size = |cell: &Cell<GiftCollectionCatalog>| {
                cell.get()
                    .and_then(|value| value.as_ref().ok())
                    .map_or(0, |c| c.models.len() + c.backdrops.len() + c.patterns.len())
            };
            while index
                .collections
                .iter()
                .map(|(_, cell)| size(cell))
                .sum::<usize>()
                > 10_000
            {
                index.collections.pop_front();
            }
        }
        let mut result = result?;
        result.cursor = reader.cursor()?;
        result.sync =
            crate::protocol::sync(reader.status()).map_err(|e| Status::data_loss(e.to_string()))?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_hub_protocol::wire as w;
    fn change(entity: w::entity::Entity) -> w::Change {
        w::Change {
            change: Some(w::change::Change::Upsert(w::Entity {
                entity: Some(entity),
            })),
        }
    }
    #[test]
    fn price_updates_reuse_projections_metadata_and_directory_changes_rebuild_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = market_replica::ReplicaWriter::open(dir.path(), "cache").unwrap();
        let mut seed = writer.begin_snapshot().unwrap();
        let order = |price: &str| {
            change(w::entity::Entity::Order(w::Order {
                id: "order".into(),
                book: Some(w::BookKey {
                    item_type_id: "gift".into(),
                    currency_id: "GRAM".into(),
                    venue_id: "venue".into(),
                }),
                mode: w::OrderMode::Ask as i32,
                price: price.into(),
                quantity: "1".into(),
                ..Default::default()
            }))
        };
        let option = |name: &str| {
            change(w::entity::Entity::AttributeOption(w::AttributeOption {
                id: "model".into(),
                attribute_id: crate::protocol::MODEL.into(),
                name: name.into(),
                ..Default::default()
            }))
        };
        seed.append(&[
            change(w::entity::Entity::ItemType(w::ItemType {
                id: "gift".into(),
                name: "Gift".into(),
                quantity_step: "1".into(),
                attributes: [(
                    crate::protocol::MODEL.into(),
                    w::AttributeRule {
                        required: true,
                        allowed_values: Some(w::AllowedAttributeValues {
                            ids: vec!["model".into()],
                        }),
                    },
                )]
                .into(),
                ..Default::default()
            })),
            option("Original"),
            order("1"),
        ])
        .unwrap();
        writer.install_snapshot(seed.finish("c0").unwrap()).unwrap();
        let cache = ProjectionCache::default();
        let first = cache.bootstrap(writer.reader()).unwrap();
        let details = cache.collection(writer.reader(), "gift".into()).unwrap();
        writer
            .apply(w::Batch {
                cursor: "c1".into(),
                changes: vec![order("2")],
            })
            .unwrap();
        let hit = cache.bootstrap(writer.reader()).unwrap();
        let detail_hit = cache.collection(writer.reader(), "gift".into()).unwrap();
        assert_eq!(hit.cursor, "c1");
        assert!(Arc::ptr_eq(&first.collections, &hit.collections));
        assert!(Arc::ptr_eq(&details.models, &detail_hit.models));
        writer
            .apply(w::Batch {
                cursor: "c2".into(),
                changes: vec![option("Renamed")],
            })
            .unwrap();
        let updated = cache.collection(writer.reader(), "gift".into()).unwrap();
        assert!(!Arc::ptr_eq(&details.models, &updated.models));
        assert_eq!(updated.models[0].name, "Renamed");
        let before = cache.bootstrap(writer.reader()).unwrap();
        writer
            .apply(w::Batch {
                cursor: "c3".into(),
                changes: vec![w::Change {
                    change: Some(w::change::Change::Remove(w::Removal {
                        key: Some(w::Key {
                            kind: w::EntityKind::Order as i32,
                            venue_id: "venue".into(),
                            id: "order".into(),
                        }),
                        ..Default::default()
                    })),
                }],
            })
            .unwrap();
        let after = cache.bootstrap(writer.reader()).unwrap();
        assert_eq!(before.books.len(), 1);
        assert!(after.books.is_empty());
        assert!(!Arc::ptr_eq(&before.books, &after.books));
        assert!(cache.collection(writer.reader(), "missing".into()).is_err());
        assert!(
            cache
                .0
                .lock()
                .unwrap()
                .collections
                .iter()
                .all(|(id, _)| id.as_str() != "missing")
        );
    }
}
