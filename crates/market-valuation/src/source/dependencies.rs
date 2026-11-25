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
pub(crate) const METADATA_KEY_LIMIT: usize = 50_000;
#[derive(Clone)]
pub struct ReadDependencies(Arc<Dependencies>);
struct Dependencies {
    books: Vec<(w::ReadRequest, ReadRevision)>,
    metadata: MetadataDependencies,
    collections: Option<MetadataRevision>,
    checked: Mutex<MetadataRevision>,
    sync: SyncStatus,
    key_count: usize,
}
impl ReadDependencies {
    pub(crate) fn key_count(&self) -> usize {
        self.0.key_count
    }
}
impl ValuationSnapshot {
    /// Validate cached projections against this pinned view on a read worker.
    pub fn dependencies_current(&self, dependencies: &ReadDependencies) -> Result<bool> {
        self.dependencies_match(dependencies, true)
    }
    /// Floor prices depend on book/catalog data; availability is projected separately.
    pub(crate) fn dependencies_data_current(
        &self,
        dependencies: &ReadDependencies,
    ) -> Result<bool> {
        self.dependencies_match(dependencies, false)
    }
    fn dependencies_match(
        &self,
        dependencies: &ReadDependencies,
        include_status: bool,
    ) -> Result<bool> {
        self.check_cancelled()?;
        let deps = &dependencies.0;
        if !self.books_current(deps, include_status)? {
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
    fn books_current(&self, deps: &Dependencies, include_status: bool) -> Result<bool> {
        let sync = self.sync()?;
        if include_status && (sync.ready != deps.sync.ready || sync.state != deps.sync.state) {
            return Ok(false);
        }
        if let Some(expected) = &deps.collections
            && *expected != self.reader.metadata_revision(&[w::EntityKind::ItemType])?
        {
            return Ok(false);
        }
        for (request, expected) in &deps.books {
            self.check_cancelled()?;
            if !expected.same_data(&self.reader.read_revision(request)?) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn dependencies(
        &self,
        scopes: impl IntoIterator<Item = BookScope>,
        currency: &CurrencyId,
        kinds: &[GiftTraitKind],
        lookups: &[ModelLookup],
    ) -> Result<ReadDependencies> {
        self.dependencies_with_books(scopes, currency, kinds, lookups, true)
    }
    pub(crate) fn target_dependencies(
        &self,
        scopes: impl IntoIterator<Item = BookScope>,
        currency: &CurrencyId,
        lookup: Option<&market_gifts_hub::input::GiftLookupInput>,
    ) -> Result<ReadDependencies> {
        let scopes: Vec<_> = scopes.into_iter().collect();
        self.dependencies_with_books(
            scopes.clone(),
            currency,
            &[
                GiftTraitKind::Model,
                GiftTraitKind::Backdrop,
                GiftTraitKind::Pattern,
            ],
            &[],
            false,
        )
        .and_then(|mut dependencies| {
            if lookup.is_some() {
                let deps = Arc::get_mut(&mut dependencies.0).expect("new dependencies");
                for scope in &scopes {
                    for mode in [w::OrderMode::Ask, w::OrderMode::Executed] {
                        let mut request = super::orders::request(scope);
                        let Some(w::selection::Source::Scope(selected)) = request
                            .selection
                            .as_mut()
                            .and_then(|selection| selection.source.as_mut())
                        else {
                            unreachable!("book scope")
                        };
                        selected.currency_id = None;
                        selected.modes = vec![mode as i32];
                        request.sort = w::ReadSort::Identity as i32;
                        let revision = self.reader.read_revision(&request)?;
                        deps.books.push((request, revision));
                    }
                }
            }
            if matches!(lookup, Some(GiftLookup::Slug(_))) {
                Arc::get_mut(&mut dependencies.0)
                    .expect("new dependencies")
                    .collections = Some(self.reader.metadata_revision(&[w::EntityKind::ItemType])?);
            }
            Ok(dependencies)
        })
    }
    fn dependencies_with_books(
        &self,
        scopes: impl IntoIterator<Item = BookScope>,
        currency: &CurrencyId,
        kinds: &[GiftTraitKind],
        lookups: &[ModelLookup],
        include_books: bool,
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
            if include_books {
                let request = super::orders::request(&scope);
                let revision = self.reader.read_revision(&request)?;
                books.push((request, revision));
            }
            keys.push(w::Key {
                kind: w::EntityKind::ItemType as i32,
                id: scope.collection.to_string(),
                ..Default::default()
            });
            if !kinds.is_empty()
                && let Some(w::entity::Entity::ItemType(item)) =
                    self.entity(w::EntityKind::ItemType, &scope.collection)?
            {
                let rules = market_gifts_hub::CollectionChoices::new(&item)
                    .map_err(|e| Status::unavailable(e.to_string()))?;
                for kind in kinds.iter().copied().collect::<BTreeSet<_>>() {
                    if let Some(ids) = rules.trait_option_ids(kind) {
                        keys.extend(ids.into_iter().map(|id| w::Key {
                            kind: w::EntityKind::AttributeOption as i32,
                            id,
                            ..Default::default()
                        }));
                    } else {
                        attributes.push(market_gifts_hub::trait_attribute(kind).into());
                    }
                }
            }
            if keys.len() > METADATA_KEY_LIMIT {
                return Err(Status::resource_exhausted(
                    "read dependency budget exceeded",
                ));
            }
        }
        Ok(ReadDependencies(Arc::new(Dependencies {
            books,
            key_count: keys.len(),
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
        if !snapshot.books_current(deps, true)? {
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
        if current.reader.metadata_revision(KINDS)? != stamp
            || !current.books_current(deps, true)?
        {
            return Ok(false);
        }
        *deps.checked.lock().unwrap() = stamp;
        Ok(true)
    }
}

/// Opaque internal observation dependency; it never appears in public reports.
#[derive(Clone)]
pub(crate) struct OrderDependency {
    request: w::ReadRequest,
    revision: ReadRevision,
}
impl OrderDependency {
    pub(crate) fn same_query(&self, other: &Self) -> bool {
        self.request == other.request
    }
    pub(crate) fn current(&self, snapshot: &ValuationSnapshot) -> Result<bool> {
        snapshot.check_cancelled()?;
        Ok(self
            .revision
            .same_data(&snapshot.reader.read_revision(&self.request)?))
    }
}
impl ValuationSnapshot {
    pub(crate) fn order_dependency(
        &self,
        query: &ResolvedGiftOrderQuery,
    ) -> Result<OrderDependency> {
        let request = market_gifts_hub::query::compile(query)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let revision = self.reader.read_revision(&request)?;
        Ok(OrderDependency { request, revision })
    }
}
