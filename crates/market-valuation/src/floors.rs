//! Lightweight collection summaries and independent trait floors; combinations are read on demand.
use crate::{FloorValuation, ValuationIssueCode, ValuationMetric};
use crate::{
    ValuationError,
    source::{BookScope, CatalogTrait, ReadDependencies, ValuationSnapshot},
};
use market_gifts::*;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::time::Instant;

const STAT_LIMIT: usize = 20_000;
const GIFT_MEMBERSHIP_LIMIT: usize = 200_000;
mod cache;
pub(crate) use cache::{BookCache, StatisticsRead};
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
pub(crate) const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Default, Clone)]
pub(crate) struct Floor {
    pub price: Option<Amount>,
    pub count: u32,
    pub observations: u32,
    identities: HashSet<Arc<GiftId>>,
}
impl Floor {
    /// Return the number of new identity memberships for bounded evidence accounting.
    pub(crate) fn observe(&mut self, price: &Amount, gift: Option<&Arc<GiftId>>) -> usize {
        self.observations += 1;
        if self
            .price
            .as_ref()
            .is_none_or(|old| price.decimal() < old.decimal())
        {
            self.price = Some(price.clone());
        }
        let added = gift.is_some_and(|id| self.identities.insert(id.clone()));
        self.count += u32::from(added);
        usize::from(added)
    }
    pub fn valuation(&self, present: bool) -> FloorValuation {
        FloorValuation {
            price: self.metric(present),
            count: self.count,
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
    asks: Option<crate::source::AskBookSnapshot>,
    preparation: Preparation,
    dependencies: ReadDependencies,
    expires_at_ms: i64,
}
impl Statistics {
    /// On-demand pair observations share the cached floors' book view and eligibility time.
    pub(crate) fn pair_floor(
        &self,
        snapshot: &ValuationSnapshot,
        model: &ModelId,
        backdrop: &BackdropId,
    ) -> Result<crate::FloorValuation, ValuationError> {
        let asks = self
            .asks
            .as_ref()
            .expect("trait statistics retain ask partitions");
        let mut floor = Floor::default();
        let mut budget = market_replica::catalog_read::ScanBudget::default();
        for (scanned, quote) in asks
            .pair_quotes(model, backdrop, self.evaluated_at.as_millisecond())
            .enumerate()
        {
            snapshot.check_cancelled()?;
            if scanned % 500 == 0 {
                budget.page()?;
            }
            if let Some(quote) = quote? {
                if floor.observations >= 50_000 {
                    return Err(ValuationError::capacity("ask observation budget exceeded"));
                }
                floor.observe(&quote.price, quote.gift_id.map(Arc::new).as_ref());
            }
        }
        Ok(floor.valuation(true))
    }
    pub(crate) fn metadata_keys(&self) -> usize {
        self.dependencies.key_count()
    }
    pub(crate) fn traits_prepared(&self) -> bool {
        self.preparation == Preparation::Traits
    }
    pub(crate) fn selection(&self, selection: FloorSelection<'_>) -> crate::FloorValuation {
        let floor = match selection {
            FloorSelection::Collection => Some(&self.collection),
            FloorSelection::Model(id) => self.models.get(id),
            FloorSelection::Backdrop(id) => self.backdrops.get(id),
            FloorSelection::Pattern(id) => self.patterns.get(id),
        };
        let empty = Floor::default();
        let floor = floor.unwrap_or(&empty);
        floor.valuation(true)
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
            asks: None,
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
    /// Immutable observations can be validated independently of cache ownership.
    pub(crate) fn data_current(
        &self,
        snapshot: &ValuationSnapshot,
    ) -> Result<bool, ValuationError> {
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
            asks: (preparation == Preparation::Traits)
                .then(|| snapshot.ask_book(scope))
                .transpose()?,
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
        let now = stats.evaluated_at.as_millisecond();
        let asks = match stats.asks.take() {
            Some(asks) => asks,
            None => snapshot.ask_book(scope)?,
        };
        let mut budget = market_replica::catalog_read::ScanBudget::default();
        let mut memberships = 0;
        for (scanned, quote) in asks.quotes(now).enumerate() {
            snapshot.check_cancelled()?;
            if scanned % 500 == 0 {
                budget.page()?;
            }
            let Some(quote) = quote? else { continue };
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
        // Identity sets are scratch space; only counts and prices survive preparation.
        for floor in std::iter::once(&mut stats.collection)
            .chain(stats.models.values_mut())
            .chain(stats.backdrops.values_mut())
            .chain(stats.patterns.values_mut())
        {
            floor.identities = HashSet::new();
        }
        if preparation == Preparation::Traits {
            stats.asks = Some(asks);
        }
        tracing::debug!(collection=%scope.collection,currency=%scope.currency,?preparation,rows=stats.collection.observations,models=stats.models.len(),backdrops=stats.backdrops.len(),patterns=stats.patterns.len(),elapsed_ms=started.elapsed().as_millis(),"prepared valuation floors");
        Ok(stats)
    }
}
fn capacity() -> ValuationError {
    ValuationError::capacity("Gift valuation statistics budget exceeded")
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
mod tests {
    use super::*;
    #[test]
    fn repeated_listings_keep_price_evidence_without_multiplying_gift_count() {
        let mut floor = Floor::default();
        let gift = Arc::new(GiftId::new("same-gift").unwrap());
        for price in ["10", "9", "11"] {
            floor.observe(&price.into(), Some(&gift));
        }
        assert_eq!(floor.count, 1);
        assert_eq!(floor.observations, 3);
        assert_eq!(floor.price.unwrap().as_str(), "9");
    }
}
