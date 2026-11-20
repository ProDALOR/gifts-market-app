//! One bounded pass supplies all components; cached data follows revisions and listing expiry.
use crate::{FloorEvidence, ValuationIssueCode, ValuationMetric};
use crate::{
    ValuationError,
    source::{BookScope, CatalogTrait, ReadDependencies, ValuationSnapshot},
};
use market_gifts::*;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

const STAT_LIMIT: usize = 20_000;
const GIFT_MEMBERSHIP_LIMIT: usize = 200_000;
const CACHE_LIMIT: usize = 8;
const MAX_AGE: Duration = Duration::from_secs(300);

#[derive(Default, Clone)]
pub(crate) struct Floor {
    pub price: Option<Amount>,
    evidence: FloorEvidence,
    identities: BTreeSet<Arc<GiftId>>,
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
            identities: BTreeSet::new(),
        }
    }
}
pub(crate) struct Statistics {
    pub evaluated_at: jiff::Timestamp,
    pub known_models: BTreeSet<ModelId>,
    pub known_backdrops: BTreeSet<BackdropId>,
    pub collection: Floor,
    pub models: BTreeMap<ModelId, Floor>,
    pub backdrops: BTreeMap<BackdropId, Floor>,
    pub pairs: BTreeMap<(ModelId, BackdropId), Floor>,
    dependencies: ReadDependencies,
    created: Instant,
    expires_at_ms: i64,
}
impl Statistics {
    pub(crate) fn deadline(&self) -> tokio::time::Instant {
        let age = MAX_AGE.saturating_sub(self.created.elapsed());
        let expiry = std::time::Duration::from_millis(
            self.expires_at_ms
                .saturating_sub(jiff::Timestamp::now().as_millisecond())
                .max(0) as u64,
        );
        tokio::time::Instant::now() + age.min(expiry)
    }

    pub(crate) fn selection(
        &self,
        model: Option<&ModelId>,
        backdrop: Option<&BackdropId>,
    ) -> (crate::ValuationMetric, FloorEvidence) {
        let floor = match (model, backdrop) {
            (None, None) => Some(&self.collection),
            (Some(model), None) => self.models.get(model),
            (None, Some(backdrop)) => self.backdrops.get(backdrop),
            (Some(model), Some(backdrop)) => self.pairs.get(&(model.clone(), backdrop.clone())),
        };
        let empty = Floor::default();
        let floor = floor.unwrap_or(&empty);
        (floor.metric(true), floor.evidence())
    }
    pub fn current(&self, snapshot: &ValuationSnapshot) -> Result<bool, ValuationError> {
        Ok(self.created.elapsed() < MAX_AGE
            && snapshot.dependencies_data_current(&self.dependencies)?
            && self.created.elapsed() < MAX_AGE
            && jiff::Timestamp::now().as_millisecond() < self.expires_at_ms)
    }
    pub fn read(snapshot: &ValuationSnapshot, scope: &BookScope) -> Result<Self, ValuationError> {
        snapshot.require_currency(&scope.currency)?;
        let mut stats = Self {
            evaluated_at: jiff::Timestamp::now(),
            known_models: snapshot.model_ids(&scope.collection)?,
            known_backdrops: snapshot.backdrop_ids(&scope.collection)?,
            collection: Floor::default(),
            models: BTreeMap::new(),
            backdrops: BTreeMap::new(),
            pairs: BTreeMap::new(),
            dependencies: snapshot.dependencies(
                [scope.clone()],
                &scope.currency,
                &[
                    CatalogTrait::Model,
                    CatalogTrait::Backdrop,
                    CatalogTrait::Pattern,
                ],
                &[],
            )?,
            created: Instant::now(),
            expires_at_ms: i64::MAX,
        };
        if stats.known_models.len() + stats.known_backdrops.len() > STAT_LIMIT {
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
                let gift = quote.gift_id.as_ref().map(|id| Arc::new(id.clone()));
                memberships += stats.collection.observe(&quote.price, gift.as_ref());
                if let Some(at) = quote.expires_at {
                    stats.expires_at_ms = stats.expires_at_ms.min(at.as_millisecond());
                }
                let model = quote
                    .model_id
                    .as_ref()
                    .filter(|id| stats.known_models.contains(*id));
                let backdrop = quote
                    .backdrop_id
                    .as_ref()
                    .filter(|id| stats.known_backdrops.contains(*id));
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
                if let Some((model, backdrop)) = model.zip(backdrop) {
                    memberships += stats
                        .pairs
                        .entry((model.clone(), backdrop.clone()))
                        .or_default()
                        .observe(&quote.price, gift.as_ref());
                }
                if memberships > GIFT_MEMBERSHIP_LIMIT
                    || stats.models.len()
                        + stats.backdrops.len()
                        + stats.pairs.len()
                        + stats.known_models.len()
                        + stats.known_backdrops.len()
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
                    .chain(stats.pairs.values_mut())
                {
                    floor.identities.clear();
                }
                stats.evaluated_at = jiff::Timestamp::now();
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
pub(crate) struct BookCache(Mutex<HashMap<BookScope, CachedBook>>);
impl BookCache {
    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<BookScope, CachedBook>>, ValuationError> {
        self.0
            .lock()
            .map_err(|_| ValuationError::unavailable("Valuation cache is unavailable"))
    }
    pub fn get(
        &self,
        snapshot: &ValuationSnapshot,
        scope: &BookScope,
    ) -> Result<Arc<Statistics>, ValuationError> {
        let cached = {
            let mut books = self.lock()?;
            books.retain(|_, entry| entry.used.elapsed() < MAX_AGE);
            books.get_mut(scope).map(|entry| {
                entry.used = Instant::now();
                entry.stats.clone()
            })
        };
        if let Some(cached) = cached
            && cached.current(snapshot)?
        {
            return Ok(cached);
        }
        // ReadExecutor serializes heavy preparation and pins after admission. No cache
        // lock spans dependency validation, catalog access or the orderbook traversal.
        let stats = Arc::new(Statistics::read(snapshot, scope)?);
        snapshot.check_cancelled()?;
        let mut books = self.lock()?;
        if !books.contains_key(scope) && books.len() >= CACHE_LIMIT {
            let oldest = books
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(scope, _)| scope.clone())
                .unwrap();
            books.remove(&oldest);
        }
        books.insert(
            scope.clone(),
            CachedBook {
                stats: stats.clone(),
                used: Instant::now(),
            },
        );
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
