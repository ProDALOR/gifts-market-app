//! Read-only market snapshots and bounded execution, independent of valuation strategies.
pub use crate::catalog::ResolvedModel;
use crate::{
    error::{DataError, Result},
    runtime::{ActorEnvironment, QueryBudget},
};
use market_gifts::*;
use market_hub_protocol::wire as w;
use market_replica::{MetadataDependencies, MetadataRevision, ReadRevision, ReplicaReader};
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::Semaphore;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BookScope {
    pub collection: CollectionId,
    pub currency: CurrencyId,
}
pub use market_gifts::AskQuote;
pub struct CatalogTraits {
    pub models: BTreeSet<ModelId>,
    pub backdrops: BTreeSet<BackdropId>,
}
/// Opaque dependencies, retaining selected metadata only; never an entire replica view.
pub struct ReadDependencies {
    books: Vec<(BookScope, ReadRevision)>,
    metadata: MetadataDependencies,
    collections: Option<MetadataRevision>,
    sync: SyncStatus,
}
#[derive(Clone)]
pub struct OrderbookReader {
    reader: ReplicaReader,
    environment: ActorEnvironment,
    budget: QueryBudget,
    pub(crate) workers: Arc<Semaphore>,
}
#[derive(Clone)]
pub struct OrderbookSnapshot {
    reader: ReplicaReader,
}
fn request(scope: &BookScope) -> w::ReadRequest {
    w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(w::Scope {
                item_type_id: Some(scope.collection.to_string()),
                currency_id: Some(scope.currency.to_string()),
                modes: vec![w::OrderMode::Ask as i32],
                ..Default::default()
            })),
            ..Default::default()
        }),
        mode: w::ReadMode::Live as i32,
        sort: w::ReadSort::PriceAsc as i32,
        limit: 500,
        ..Default::default()
    }
}
impl OrderbookReader {
    pub(crate) fn new(
        reader: ReplicaReader,
        environment: ActorEnvironment,
        budget: QueryBudget,
    ) -> Self {
        Self {
            reader,
            environment,
            budget,
            workers: Arc::new(Semaphore::new(1)),
        }
    }
    pub fn snapshot(&self) -> Result<OrderbookSnapshot> {
        Ok(OrderbookSnapshot {
            reader: self.reader.snapshot()?,
        })
    }
    pub fn is_current(&self, dependencies: &ReadDependencies) -> bool {
        self.snapshot()
            .and_then(|snapshot| {
                let sync = snapshot.sync()?;
                if sync.ready != dependencies.sync.ready
                    || sync.state != dependencies.sync.state
                    || !snapshot.reader.metadata_matches(&dependencies.metadata)?
                {
                    return Ok(false);
                }
                if let Some(expected) = &dependencies.collections
                    && *expected
                        != snapshot
                            .reader
                            .metadata_revision(&[w::EntityKind::ItemType])?
                {
                    return Ok(false);
                }
                for (scope, expected) in &dependencies.books {
                    if !expected.same_data(&snapshot.reader.read_revision(&request(scope))?) {
                        return Ok(false);
                    }
                }
                Ok(true)
            })
            .unwrap_or(false)
    }
    /// Cancelled work retains its capacity until the CPU worker actually exits.
    pub async fn prepare<T: Send + 'static>(
        &self,
        work: impl FnOnce(OrderbookSnapshot) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let _admission = self.environment.jobs.token();
        let heavy = tokio::select! { biased;
            _=self.environment.stop.cancelled()=>return Err(DataError::unavailable("Market data service is stopping")),
            permit=self.workers.clone().acquire_owned()=>permit.map_err(|_|DataError::unavailable("Read workers closed"))?,
        };
        let permit = self.budget.wait().await?;
        let snapshot = self.snapshot()?;
        self.environment
            .jobs
            .spawn_blocking_on(
                move || {
                    let (_heavy, _permit) = (heavy, permit);
                    work(snapshot)
                },
                &self.environment.runtime,
            )
            .await
            .map_err(|_| DataError::unavailable("Market read worker stopped"))?
    }
}
impl OrderbookSnapshot {
    pub fn revision(&self) -> Result<String> {
        Ok(self.reader.cursor()?)
    }
    pub fn sync(&self) -> Result<SyncStatus> {
        crate::protocol::sync(self.reader.status())
    }
    pub fn orders(&self, query: GiftOrdersQuery) -> Result<Page<GiftOrder>> {
        let request = crate::query::orders(query, w::ReadMode::Live)?;
        crate::protocol::orders(self.reader.read(request)?)
    }
    pub fn resolve_models(
        &self,
        positions: Vec<valuation::ModelPosition>,
    ) -> Result<Vec<ResolvedModel>> {
        Ok(crate::catalog::resolve_models(&self.reader, positions)?)
    }
    fn entity(&self, kind: w::EntityKind, id: &str) -> Result<Option<w::entity::Entity>> {
        Ok(
            crate::catalog::projection::entities(&self.reader, kind, Some(&[id.to_owned()]))?
                .into_iter()
                .next(),
        )
    }
    pub fn require_currency(&self, currency: &CurrencyId) -> Result<()> {
        self.entity(w::EntityKind::Currency, currency)?
            .ok_or_else(|| DataError::invalid("unknown quote currency"))?;
        Ok(())
    }
    pub fn traits(&self, collection: &CollectionId) -> Result<CatalogTraits> {
        let Some(w::entity::Entity::ItemType(item)) =
            self.entity(w::EntityKind::ItemType, collection)?
        else {
            return Err(DataError::invalid("unknown gift collection"));
        };
        if !market_gifts_hub::is_gift(&item) {
            return Err(DataError::invalid("unknown gift collection"));
        }
        let rules = market_gifts_hub::CollectionChoices::new(&item)
            .map_err(|e| DataError::unavailable(e.to_string()))?;
        let ids = rules.option_ids();
        let unrestricted = rules.unrestricted_attributes().iter().any(|id| {
            matches!(
                id.as_str(),
                market_gifts_hub::MODEL | market_gifts_hub::BACKDROP
            )
        });
        let options = crate::catalog::projection::entities(
            &self.reader,
            w::EntityKind::AttributeOption,
            if unrestricted { None } else { Some(&ids) },
        )?;
        let mut result = CatalogTraits {
            models: BTreeSet::new(),
            backdrops: BTreeSet::new(),
        };
        for value in options {
            if let w::entity::Entity::AttributeOption(option) = value
                && rules.accepts(&option)
            {
                match option.attribute_id.as_str() {
                    market_gifts_hub::MODEL => {
                        result.models.insert(
                            option
                                .id
                                .try_into()
                                .map_err(|e| DataError::unavailable(format!("{e}")))?,
                        );
                    }
                    market_gifts_hub::BACKDROP => {
                        result.backdrops.insert(
                            option
                                .id
                                .try_into()
                                .map_err(|e| DataError::unavailable(format!("{e}")))?,
                        );
                    }
                    _ => {}
                }
            }
        }
        Ok(result)
    }
    /// Sorted eligible asks over this pinned view. The caller owns its scan budget.
    pub fn asks(
        &self,
        scope: &BookScope,
        now_ms: i64,
        page_token: String,
    ) -> Result<Page<AskQuote>> {
        let mut query = request(scope);
        query.page_token = page_token;
        let page = self.reader.read(query)?.page;
        let mut data = Vec::new();
        for entity in page.entities {
            if let Some(w::entity::Entity::Order(order)) = entity.entity
                && let Some(quote) = market_gifts_hub::ask_quote(order, now_ms)
                    .map_err(|e| DataError::unavailable(e.to_string()))?
            {
                data.push(quote);
            }
        }
        Ok(Page {
            data,
            next_page_token: page.next_page_token,
            cursor: page.cursor,
            sync: self.sync()?,
        })
    }
    pub fn dependencies(
        &self,
        scopes: impl IntoIterator<Item = BookScope>,
        currency: &CurrencyId,
        resolve_names: bool,
    ) -> Result<ReadDependencies> {
        let scopes = scopes.into_iter().collect::<BTreeSet<_>>();
        let mut keys = vec![w::Key {
            kind: w::EntityKind::Currency as i32,
            id: currency.to_string(),
            ..Default::default()
        }];
        let mut attributes = Vec::new();
        let mut books = Vec::new();
        for scope in scopes {
            books.push((scope.clone(), self.reader.read_revision(&request(&scope))?));
            keys.push(w::Key {
                kind: w::EntityKind::ItemType as i32,
                id: scope.collection.to_string(),
                ..Default::default()
            });
            if let Some(w::entity::Entity::ItemType(item)) =
                self.entity(w::EntityKind::ItemType, &scope.collection)?
            {
                let rules = market_gifts_hub::CollectionChoices::new(&item)
                    .map_err(|e| DataError::unavailable(e.to_string()))?;
                attributes.extend(rules.unrestricted_attributes());
                keys.extend(rules.option_ids().into_iter().map(|id| w::Key {
                    kind: w::EntityKind::AttributeOption as i32,
                    id,
                    ..Default::default()
                }));
            }
            if keys.len() > 50_000 {
                return Err(DataError::capacity("read dependency budget exceeded"));
            }
        }
        Ok(ReadDependencies {
            books,
            metadata: self.reader.metadata_dependencies(keys, attributes)?,
            collections: resolve_names
                .then(|| self.reader.metadata_revision(&[w::EntityKind::ItemType]))
                .transpose()?,
            sync: self.sync()?,
        })
    }
}
