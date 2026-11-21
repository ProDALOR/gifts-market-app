//! Lightweight collection summaries and independent trait floors; combinations are read on demand.
use crate::{FloorEvidence, ValuationIssueCode, ValuationMetric};
use crate::{
    ValuationError,
    source::{BookScope, CatalogTrait, ReadDependencies, ValuationSnapshot},
};
use market_gifts::*;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

const STAT_LIMIT: usize = 20_000;
const GIFT_MEMBERSHIP_LIMIT: usize = 200_000;
const TRAIT_CACHE_LIMIT: usize = 8;
const COLLECTION_CACHE_LIMIT: usize = 512;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Preparation {
    Collection,
    Traits,
}
pub(crate) enum FloorSelection<'a> {
    Collection,
    Model(&'a ModelId),
    Backdrop(&'a BackdropId),
    Pattern(&'a PatternId),
}
pub(crate) const RECHECK_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Default, Clone)]
pub(crate) struct Floor {
    pub price: Option<Amount>,
    evidence: FloorEvidence,
    identities: HashSet<Arc<GiftId>>,
}
impl Floor {
    /// Return the number of new identity memberships for bounded evidence accounting.
    fn observe(&mut self, price: &Amount, gift: Option<&Arc<GiftId>>) -> usize {
        self.evidence.listings += 1;
        if self
            .price
            .as_ref()
            .is_none_or(|old| price.decimal() < old.decimal())
        {
            self.price = Some(price.clone());
        }
        let added = gift.is_some_and(|id| self.identities.insert(id.clone()));
        self.evidence.gifts += u32::from(added);
        usize::from(added)
    }
    pub fn from_valuation(value: &crate::AskFloorValuation) -> Self {
        Self {
            price: value.price.value().cloned(),
            evidence: value.evidence.clone(),
            identities: HashSet::new(),
        }
    }
    pub fn evidence(&self) -> FloorEvidence {
        self.evidence.clone()
    }
    #[cfg(test)]
    pub fn fixture(price: &str, gifts: u32) -> Self {
        Self {
            price: Some(Amount::new(price).unwrap()),
            evidence: FloorEvidence {
                listings: gifts,
                gifts,
            },
            identities: HashSet::new(),
        }
    }
}
pub(crate) struct Statistics {
    pub(crate) scope: BookScope,
    pub evaluated_at: jiff::Timestamp,
    pub known_models: BTreeSet<ModelId>,
    pub known_backdrops: BTreeSet<BackdropId>,
    pub known_patterns: BTreeSet<PatternId>,
    pub collection: Floor,
    pub models: BTreeMap<ModelId, Floor>,
    pub backdrops: BTreeMap<BackdropId, Floor>,
    pub patterns: BTreeMap<PatternId, Floor>,
    preparation: Preparation,
    dependencies: ReadDependencies,
    expires_at_ms: i64,
}
impl Statistics {
    pub(crate) fn metadata_keys(&self) -> usize {
        self.dependencies.key_count()
    }
    pub(crate) fn deadline(&self) -> tokio::time::Instant {
        let expiry = std::time::Duration::from_millis(
            self.expires_at_ms
                .saturating_sub(jiff::Timestamp::now().as_millisecond())
                .max(0) as u64,
        );
        tokio::time::Instant::now() + RECHECK_INTERVAL.min(expiry)
    }

    pub(crate) fn traits_prepared(&self) -> bool {
        self.preparation == Preparation::Traits
    }
    pub(crate) fn selection(&self, selection: FloorSelection<'_>) -> crate::AskFloorValuation {
        let floor = match selection {
            FloorSelection::Collection => Some(&self.collection),
            FloorSelection::Model(id) => self.models.get(id),
            FloorSelection::Backdrop(id) => self.backdrops.get(id),
            FloorSelection::Pattern(id) => self.patterns.get(id),
        };
        let empty = Floor::default();
        let floor = floor.unwrap_or(&empty);
        crate::AskFloorValuation {
            price: floor.metric(true),
            evidence: floor.evidence(),
        }
    }
    fn collection_only(&self, snapshot: &ValuationSnapshot) -> Result<Self, ValuationError> {
        Ok(Self {
            scope: self.scope.clone(),
            evaluated_at: self.evaluated_at,
            known_models: BTreeSet::new(),
            known_backdrops: BTreeSet::new(),
            known_patterns: BTreeSet::new(),
            collection: self.collection.clone(),
            models: BTreeMap::new(),
            backdrops: BTreeMap::new(),
            patterns: BTreeMap::new(),
            preparation: Preparation::Collection,
            dependencies: snapshot.dependencies(
                [self.scope.clone()],
                &self.scope.currency,
                &[],
                &[],
            )?,
            expires_at_ms: self.expires_at_ms,
        })
    }
    pub fn current(&self, snapshot: &ValuationSnapshot) -> Result<bool, ValuationError> {
        Ok(jiff::Timestamp::now().as_millisecond() < self.expires_at_ms
            && snapshot.dependencies_data_current(&self.dependencies)?
            && jiff::Timestamp::now().as_millisecond() < self.expires_at_ms)
    }
    pub fn read(
        snapshot: &ValuationSnapshot,
        scope: &BookScope,
        preparation: Preparation,
    ) -> Result<Self, ValuationError> {
        let started = Instant::now();
        snapshot.require_currency(&scope.currency)?;
        let mut stats = Self {
            scope: scope.clone(),
            evaluated_at: jiff::Timestamp::now(),
            known_models: if preparation == Preparation::Traits {
                snapshot.model_ids(&scope.collection)?
            } else {
                BTreeSet::new()
            },
            known_backdrops: if preparation == Preparation::Traits {
                snapshot.backdrop_ids(&scope.collection)?
            } else {
                BTreeSet::new()
            },
            known_patterns: if preparation == Preparation::Traits {
                snapshot.pattern_ids(&scope.collection)?
            } else {
                BTreeSet::new()
            },
            collection: Floor::default(),
            models: BTreeMap::new(),
            backdrops: BTreeMap::new(),
            patterns: BTreeMap::new(),
            preparation,
            dependencies: snapshot.dependencies(
                [scope.clone()],
                &scope.currency,
                if preparation == Preparation::Traits {
                    &[
                        CatalogTrait::Model,
                        CatalogTrait::Backdrop,
                        CatalogTrait::Pattern,
                    ]
                } else {
                    &[]
                },
                &[],
            )?,
            expires_at_ms: i64::MAX,
        };
        if stats.known_models.len() + stats.known_backdrops.len() + stats.known_patterns.len()
            > STAT_LIMIT
        {
            return Err(capacity());
        }
        let now = jiff::Timestamp::now().as_millisecond();
        let mut token = String::new();
        let mut budget = market_replica::catalog_read::ScanBudget::default();
        let mut memberships = 0;
        loop {
            budget.page()?;
            let page = snapshot.asks(scope, now, token)?;
            for quote in page.data {
                snapshot.check_cancelled()?;
                let gift = quote.gift_id.map(Arc::new);
                memberships += stats.collection.observe(&quote.price, gift.as_ref());
                if let Some(at) = quote.expires_at {
                    stats.expires_at_ms = stats.expires_at_ms.min(at.as_millisecond());
                }
                if preparation == Preparation::Collection {
                    if memberships > GIFT_MEMBERSHIP_LIMIT {
                        return Err(capacity());
                    }
                    continue;
                }
                let model = quote
                    .model_id
                    .as_ref()
                    .filter(|id| stats.known_models.contains(*id));
                let backdrop = quote
                    .backdrop_id
                    .as_ref()
                    .filter(|id| stats.known_backdrops.contains(*id));
                if let Some(id) = quote
                    .pattern_id
                    .as_ref()
                    .filter(|id| stats.known_patterns.contains(*id))
                {
                    memberships += stats
                        .patterns
                        .entry(id.clone())
                        .or_default()
                        .observe(&quote.price, gift.as_ref());
                }
                if let Some(id) = model {
                    memberships += stats
                        .models
                        .entry(id.clone())
                        .or_default()
                        .observe(&quote.price, gift.as_ref());
                }
                if let Some(id) = backdrop {
                    memberships += stats
                        .backdrops
                        .entry(id.clone())
                        .or_default()
                        .observe(&quote.price, gift.as_ref());
                }
                if memberships > GIFT_MEMBERSHIP_LIMIT
                    || stats.models.len()
                        + stats.backdrops.len()
                        + stats.patterns.len()
                        + stats.known_models.len()
                        + stats.known_backdrops.len()
                        + stats.known_patterns.len()
                        > STAT_LIMIT
                {
                    return Err(capacity());
                }
            }
            if page.next_page_token.is_empty() {
                // Identity sets are traversal scratch space. Cached statistics retain
                // only prices and counts; cross-venue deduplication does not grow the cache.
                for floor in std::iter::once(&mut stats.collection)
                    .chain(stats.models.values_mut())
                    .chain(stats.backdrops.values_mut())
                    .chain(stats.patterns.values_mut())
                {
                    floor.identities = HashSet::new();
                }
                stats.evaluated_at = jiff::Timestamp::now();
                tracing::debug!(collection=%scope.collection,currency=%scope.currency,?preparation,rows=stats.collection.evidence.listings,models=stats.models.len(),backdrops=stats.backdrops.len(),patterns=stats.patterns.len(),elapsed_ms=started.elapsed().as_millis(),"prepared valuation floors");
                return Ok(stats);
            }
            token = page.next_page_token;
        }
    }
}
fn capacity() -> ValuationError {
    ValuationError::capacity("Gift valuation statistics budget exceeded")
}

struct CachedBook {
    stats: Arc<Statistics>,
    used: Instant,
}
#[derive(Default)]
struct CachedBooks {
    collections: HashMap<BookScope, CachedBook>,
    traits: HashMap<BookScope, CachedBook>,
}
#[derive(Default)]
pub(crate) struct BookCache(Mutex<CachedBooks>);
impl CachedBooks {
    fn insert(&mut self, scope: BookScope, stats: Arc<Statistics>) {
        let (books, limit) = if stats.traits_prepared() {
            (&mut self.traits, TRAIT_CACHE_LIMIT)
        } else {
            (&mut self.collections, COLLECTION_CACHE_LIMIT)
        };
        if !books.contains_key(&scope) && books.len() >= limit {
            if let Some(oldest) = books
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(scope, _)| scope.clone())
            {
                books.remove(&oldest);
            }
        }
        books.insert(
            scope,
            CachedBook {
                stats,
                used: Instant::now(),
            },
        );
    }
}
impl BookCache {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, CachedBooks>, ValuationError> {
        self.0
            .lock()
            .map_err(|_| ValuationError::unavailable("Valuation cache is unavailable"))
    }
    pub fn get(
        &self,
        snapshot: &ValuationSnapshot,
        scope: &BookScope,
    ) -> Result<Arc<Statistics>, ValuationError> {
        self.prepare(snapshot, scope, Preparation::Traits)
    }
    pub fn collection(
        &self,
        snapshot: &ValuationSnapshot,
        scope: &BookScope,
    ) -> Result<Arc<Statistics>, ValuationError> {
        self.prepare(snapshot, scope, Preparation::Collection)
    }
    fn prepare(
        &self,
        snapshot: &ValuationSnapshot,
        scope: &BookScope,
        preparation: Preparation,
    ) -> Result<Arc<Statistics>, ValuationError> {
        let cached = {
            let mut books = self.lock()?;
            let entries = if preparation == Preparation::Traits {
                &mut books.traits
            } else {
                &mut books.collections
            };
            entries.get_mut(scope).map(|entry| {
                entry.used = Instant::now();
                entry.stats.clone()
            })
        };
        if let Some(cached) = cached
            && cached.current(snapshot)?
        {
            return Ok(cached);
        }
        // All preparation runs on the shared heavy-read lane, pinned after admission.
        let stats = Arc::new(Statistics::read(snapshot, scope, preparation)?);
        let summary = if preparation == Preparation::Traits {
            Some(Arc::new(stats.collection_only(snapshot)?))
        } else {
            None
        };
        snapshot.check_cancelled()?;
        let mut books = self.lock()?;
        if let Some(summary) = summary {
            books.insert(scope.clone(), summary);
        }
        books.insert(scope.clone(), stats.clone());
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_listings_keep_price_evidence_without_multiplying_gift_count() {
        let mut floor = Floor::default();
        let gift = Arc::new(GiftId::new("same-gift").unwrap());
        for price in ["10", "9", "11"] {
            floor.observe(&price.into(), Some(&gift));
        }
        assert_eq!(
            floor.evidence(),
            FloorEvidence {
                listings: 3,
                gifts: 1
            }
        );
        assert_eq!(floor.price.unwrap().as_str(), "9");
    }
}

impl Floor {
    pub(crate) fn metric(&self, present: bool) -> ValuationMetric {
        if !present {
            return crate::market::unavailable(
                ValuationIssueCode::MissingTrait,
                "Gift has no resolved catalog trait",
                None,
                None,
            );
        }
        match &self.price {
            Some(value) => ValuationMetric::Value {
                value: value.clone(),
            },
            None => crate::market::unavailable(
                ValuationIssueCode::NotEnoughData,
                "No eligible asks match this selection",
                None,
                None,
            ),
        }
    }
}

#[cfg(test)]
#[path = "preparation_tests.rs"]
mod preparation_tests;
