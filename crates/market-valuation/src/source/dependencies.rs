use super::*;
use market_gifts::*;
use market_hub_protocol::wire as w;
use market_replica::{MetadataDependencies, MetadataRevision, ReadRevision};
use std::{collections::BTreeSet, sync::Mutex};
const KINDS: &[w::EntityKind] = &[
    w::EntityKind::ItemType,
    w::EntityKind::AttributeOption,
    w::EntityKind::Currency,
];
#[derive(Clone)]
pub struct ReadDependencies(Arc<Dependencies>);
struct Dependencies {
    books: Vec<(BookScope, ReadRevision)>,
    metadata: MetadataDependencies,
    collections: Option<MetadataRevision>,
    checked: Mutex<MetadataRevision>,
    sync: SyncStatus,
}
impl ValuationSnapshot {
    /// Validate cached projections against this pinned view on a read worker.
    pub fn dependencies_current(&self, dependencies: &ReadDependencies) -> Result<bool> {
        self.check_cancelled()?;
        let deps = &dependencies.0;
        if !self.books_current(deps)? {
            return Ok(false);
        }
        let stamp = self.reader.metadata_revision(KINDS)?;
        if *deps.checked.lock().unwrap() == stamp {
            return Ok(true);
        }
        let matches = self.reader.metadata_matches_with(&deps.metadata, || {
            self.check_cancelled()
                .map_err(|e| tonic::Status::cancelled(e.to_string()))
        })?;
        if matches {
            *deps.checked.lock().unwrap() = stamp;
        }
        Ok(matches)
    }
    fn books_current(&self, deps: &Dependencies) -> Result<bool> {
        let sync = self.sync()?;
        if sync.ready != deps.sync.ready || sync.state != deps.sync.state {
            return Ok(false);
        }
        if let Some(expected) = &deps.collections
            && *expected != self.reader.metadata_revision(&[w::EntityKind::ItemType])?
        {
            return Ok(false);
        }
        for (scope, expected) in &deps.books {
            self.check_cancelled()?;
            if !expected.same_data(&self.reader.read_revision(&super::orders::request(scope))?) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn dependencies(
        &self,
        scopes: impl IntoIterator<Item = BookScope>,
        currency: &CurrencyId,
        kinds: &[CatalogTrait],
        lookups: &[ModelLookup],
    ) -> Result<ReadDependencies> {
        self.check_cancelled()?;
        let scopes = scopes.into_iter().collect::<BTreeSet<_>>();
        let mut keys = vec![w::Key {
            kind: w::EntityKind::Currency as i32,
            id: currency.to_string(),
            ..Default::default()
        }];
        let mut attributes = Vec::new();
        let mut books = Vec::new();
        for scope in scopes {
            self.check_cancelled()?;
            books.push((
                scope.clone(),
                self.reader.read_revision(&super::orders::request(&scope))?,
            ));
            keys.push(w::Key {
                kind: w::EntityKind::ItemType as i32,
                id: scope.collection.to_string(),
                ..Default::default()
            });
            if let Some(w::entity::Entity::ItemType(item)) =
                self.entity(w::EntityKind::ItemType, &scope.collection)?
            {
                let rules = market_gifts_hub::CollectionChoices::new(&item)
                    .map_err(|e| Status::unavailable(e.to_string()))?;
                for attribute in kinds
                    .iter()
                    .map(|kind| kind.attribute())
                    .collect::<BTreeSet<_>>()
                {
                    if let Some(ids) = rules.trait_option_ids(attribute) {
                        keys.extend(ids.into_iter().map(|id| w::Key {
                            kind: w::EntityKind::AttributeOption as i32,
                            id,
                            ..Default::default()
                        }));
                    } else {
                        attributes.push(attribute.into());
                    }
                }
            }
            if keys.len() > 50_000 {
                return Err(Status::resource_exhausted(
                    "read dependency budget exceeded",
                ));
            }
        }
        Ok(ReadDependencies(Arc::new(Dependencies {
            books,
            metadata: self.reader.metadata_dependencies(keys, attributes)?,
            collections: lookups
                .iter()
                .any(|lookup| {
                    !matches!(
                        lookup,
                        ModelLookup::Catalog { .. } | ModelLookup::Collection { .. }
                    )
                })
                .then(|| self.reader.metadata_revision(&[w::EntityKind::ItemType]))
                .transpose()?,
            checked: Mutex::new(self.reader.metadata_revision(KINDS)?),
            sync: self.sync()?,
        })))
    }
}
impl ValuationSource {
    /// Price-only updates take the cheap revision path. Full metadata checks run off the Arbiter.
    pub async fn is_current(&self, dependencies: &ReadDependencies) -> bool {
        self.validate(dependencies).await.unwrap_or(false)
    }
    async fn validate(&self, dependencies: &ReadDependencies) -> Result<bool> {
        let snapshot = self.snapshot()?;
        let deps = &dependencies.0;
        if !snapshot.books_current(deps)? {
            return Ok(false);
        }
        let stamp = snapshot.reader.metadata_revision(KINDS)?;
        if *deps.checked.lock().unwrap() == stamp {
            return Ok(true);
        }
        let dependencies = dependencies.clone();
        let checked = self
            .executor
            .run(move |cancelled| {
                snapshot
                    .reader
                    .metadata_matches_with(&dependencies.0.metadata, || {
                        market_replica::catalog_read::check(&cancelled)
                    })
            })
            .await??;
        if !checked {
            return Ok(false);
        }
        let current = self.snapshot()?;
        if current.reader.metadata_revision(KINDS)? != stamp || !current.books_current(deps)? {
            return Ok(false);
        }
        *deps.checked.lock().unwrap() = stamp;
        Ok(true)
    }
}
