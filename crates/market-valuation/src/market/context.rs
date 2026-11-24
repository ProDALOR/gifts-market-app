//! Read access and bounded shared preparation; target/options belong to typed inputs.
use super::{ExecutionInput, execution::ExecutionStatistics};
use crate::{
    ValuationError,
    floors::{BookCache, Statistics},
    source::{BookScope, ValuationSnapshot},
};
use market_gifts::{CollectionId, CurrencyId, GiftOrder, GiftOrderQuery, OrderSource};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
type ExecutionCache = VecDeque<(
    ExecutionInput,
    Result<Arc<ExecutionStatistics>, ValuationError>,
)>;
type MatchingFloorCache = Vec<(
    super::ResolvedTarget,
    Result<crate::FloorValuation, ValuationError>,
)>;
type PairFloorCache = BTreeMap<
    (
        CollectionId,
        market_gifts::ModelId,
        market_gifts::BackdropId,
    ),
    Result<crate::FloorValuation, ValuationError>,
>;
pub struct ValuationContext<'a> {
    snapshot: &'a ValuationSnapshot,
    currency: &'a CurrencyId,
    books: &'a BookCache,
    executions: RefCell<ExecutionCache>,
    failed_books: RefCell<BTreeMap<CollectionId, ValuationError>>,
    pairs: RefCell<PairFloorCache>,
    matching_floors: RefCell<MatchingFloorCache>,
    bids: RefCell<BTreeMap<CollectionId, Result<Arc<Vec<GiftOrder>>, ValuationError>>>,
    pub(super) reads: RefCell<super::read_set::ReadSet>,
}
impl<'a> ValuationContext<'a> {
    pub(crate) fn new(
        snapshot: &'a ValuationSnapshot,
        currency: &'a CurrencyId,
        books: &'a BookCache,
    ) -> Self {
        Self {
            snapshot,
            currency,
            books,
            executions: Default::default(),
            failed_books: Default::default(),
            pairs: Default::default(),
            matching_floors: Default::default(),
            bids: Default::default(),
            reads: RefCell::new(super::read_set::ReadSet::new()),
        }
    }
    pub fn snapshot(&self) -> &ValuationSnapshot {
        self.snapshot
    }
    pub fn currency(&self) -> &CurrencyId {
        self.currency
    }
    pub(crate) fn floor_statistics(
        &self,
        collection: &CollectionId,
    ) -> Result<Arc<Statistics>, ValuationError> {
        self.snapshot.check_cancelled()?;
        if let Some(error) = self.failed_books.borrow().get(collection) {
            return Err(error.clone());
        }
        // This context already pins its observations. Do not revisit the bounded
        // global LRU (or rescan an evicted book) for another appearance in the same read.
        if let Some(stats) = self
            .reads
            .borrow()
            .statistics
            .iter()
            .find(|stats| &stats.scope.collection == collection && stats.traits_prepared())
        {
            return Ok(stats.clone());
        }
        let result = self.books.get(
            self.snapshot,
            &BookScope {
                collection: collection.clone(),
                currency: self.currency.clone(),
            },
        );
        match result {
            Ok(stats) => {
                let statistics = stats.statistics.clone();
                self.reads.borrow_mut().statistics(stats)?;
                Ok(statistics)
            }
            Err(error) => {
                self.observe(&GiftOrderQuery::new(market_gifts::GiftFilters::new(
                    collection.clone(),
                    self.currency.clone(),
                )))?;
                self.failed_books
                    .borrow_mut()
                    .insert(collection.clone(), error.clone());
                Err(error)
            }
        }
    }
    pub(crate) fn pair_floor(
        &self,
        collection: &CollectionId,
        model: &market_gifts::ModelId,
        backdrop: &market_gifts::BackdropId,
    ) -> Result<crate::FloorValuation, ValuationError> {
        self.snapshot.check_cancelled()?;
        let key = (collection.clone(), model.clone(), backdrop.clone());
        if let Some(value) = self.pairs.borrow().get(&key) {
            return value.clone();
        }
        let value = self
            .floor_statistics(collection)?
            .pair_floor(self.snapshot, model, backdrop);
        self.pairs.borrow_mut().insert(key, value.clone());
        value
    }

    pub(crate) fn collection_statistics(
        &self,
        collection: &CollectionId,
    ) -> Result<Arc<Statistics>, ValuationError> {
        self.snapshot.check_cancelled()?;
        if let Some(stats) = self
            .reads
            .borrow()
            .statistics
            .iter()
            .find(|stats| &stats.scope.collection == collection)
        {
            return Ok(stats.clone());
        }
        let stats = self.books.collection(
            self.snapshot,
            &BookScope {
                collection: collection.clone(),
                currency: self.currency.clone(),
            },
        )?;
        let statistics = stats.statistics.clone();
        self.reads.borrow_mut().statistics(stats)?;
        Ok(statistics)
    }
    /// Record the sources that the next periodic refresh needs to validate.
    pub fn orders(
        &self,
        query: &GiftOrderQuery,
        continuation: String,
    ) -> Result<crate::source::Page<GiftOrder>, ValuationError> {
        self.observe(query)?;
        let page = self.snapshot.valuation_orders(query, continuation)?;
        if query.source != OrderSource::History {
            let now = jiff::Timestamp::now();
            if let Some(expiry) = page
                .data
                .iter()
                .filter_map(|order| order.expires_at)
                .filter(|expiry| *expiry > now)
                .min()
            {
                // Expiry invalidates at the next scheduled refresh, without shortening it.
                self.reads.borrow_mut().expires_at(expiry);
            }
        }
        Ok(page)
    }

    fn observe(&self, query: &GiftOrderQuery) -> Result<(), ValuationError> {
        self.reads.borrow_mut().observe(self.snapshot, query)
    }

    /// Batches and all reference scopes share one bounded bid projection per collection.
    /// It lives only for this admitted read; completed reports retain the small winning quotes.
    pub(crate) fn bid_orders(
        &self,
        collection: &CollectionId,
    ) -> Result<Arc<Vec<GiftOrder>>, ValuationError> {
        self.snapshot.check_cancelled()?;
        if let Some(result) = self.bids.borrow().get(collection) {
            return result.clone();
        }
        let result = (|| {
            let mut query = GiftOrderQuery::new(market_gifts::GiftFilters::new(
                collection.clone(),
                self.currency.clone(),
            ));
            query.source = OrderSource::Bids;
            query.limit = 500;
            let mut token = String::new();
            let mut rows = Vec::new();
            let mut memberships = 0usize;
            let mut budget = market_replica::catalog_read::ScanBudget::default();
            loop {
                budget.page()?;
                let page = self.orders(&query, token)?;
                if rows.len() + page.data.len() > super::execution::MAX_OBSERVATIONS {
                    return Err(ValuationError::capacity("bid observation budget exceeded"));
                }
                for order in &page.data {
                    self.snapshot.check_cancelled()?;
                    if let market_gifts::GiftTarget::Criteria { criteria } = &order.target {
                        memberships += criteria.models.len()
                            + criteria.backdrops.len()
                            + criteria.patterns.len();
                        if memberships > 200_000 {
                            return Err(ValuationError::capacity("bid condition budget exceeded"));
                        }
                    }
                }
                rows.extend(page.data);
                if page.next_page_token.is_empty() {
                    break;
                }
                token = page.next_page_token;
            }
            let now = jiff::Timestamp::now();
            rows.retain(|order| {
                order.quantity > 0
                    && order.price.decimal() > rust_decimal::Decimal::ZERO
                    && order.expires_at.is_none_or(|expiry| expiry > now)
            });
            Ok(Arc::new(rows))
        })();
        self.bids
            .borrow_mut()
            .insert(collection.clone(), result.clone());
        result
    }

    pub(super) fn matching_floor(
        &self,
        target: &super::ResolvedTarget,
    ) -> Result<crate::FloorValuation, ValuationError> {
        self.snapshot.check_cancelled()?;
        if let Some((_, result)) = self
            .matching_floors
            .borrow()
            .iter()
            .find(|(key, _)| key == target)
        {
            return result.clone();
        }
        let result = super::strategies::matching_ask_floor(self, target);
        self.matching_floors
            .borrow_mut()
            .push((target.clone(), result.clone()));
        result
    }

    pub(super) fn executions(
        &self,
        input: &ExecutionInput,
    ) -> Result<Arc<ExecutionStatistics>, ValuationError> {
        self.snapshot.check_cancelled()?;
        {
            let mut cache = self.executions.borrow_mut();
            if let Some(index) = cache.iter().position(|(key, _)| key == input) {
                let entry = cache.remove(index).expect("located execution input");
                let result = entry.1.clone();
                cache.push_back(entry);
                return result;
            }
        }
        // Batches can alternate selections and periods. Bound retained samples across
        // inputs to one full scan's budget, rather than retaining a full scan per query.
        let result = ExecutionStatistics::read(self, input).map(Arc::new);
        let mut cache = self.executions.borrow_mut();
        let samples = |result: &Result<Arc<ExecutionStatistics>, ValuationError>| {
            result
                .as_ref()
                .map_or(0, |v| v.evidence.executions as usize)
        };
        let incoming = samples(&result);
        while cache.len() >= super::MAX_VALUATION_BATCH
            || cache.iter().map(|(_, v)| samples(v)).sum::<usize>() + incoming
                > super::execution::MAX_OBSERVATIONS
        {
            cache.pop_front();
        }
        cache.push_back((input.clone(), result.clone()));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExecutionPeriod;
    use crate::{
        ResolvedTarget, StrategyInput, gift_floor::tests::book, market::tests::execution,
        source::ValuationSource,
    };
    use market_gifts::Period;
    use market_hub_protocol::wire as w;
    use tokio_util::sync::CancellationToken;
    #[tokio::test]
    async fn execution_cache_is_shared_for_equal_inputs_and_scoped_to_selection_and_period() {
        let (_dir, mut writer) = book();
        writer
            .apply(w::Batch {
                cursor: "executions".into(),
                changes: (0..3)
                    .map(|i| execution(i, "10", "2026-01-10T00:00:00Z"))
                    .collect(),
            })
            .unwrap();
        let reads =
            market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
        let source = ValuationSource::new(reads.handle());
        source
            .prepare(move |snapshot| -> Result<(), ValuationError> {
                let books = BookCache::default();
                let currency = "GRAM".into();
                let context = ValuationContext::new(&snapshot, &currency, &books);
                let target = Arc::new(ResolvedTarget::Selection {
                    collection_id: "gift".into(),
                    criteria: market_gifts::GiftCriteria::default(),
                });
                let january = Period {
                    from: Some("2026-01-01T00:00:00Z".parse().unwrap()),
                    until: Some("2026-02-01T00:00:00Z".parse().unwrap()),
                };
                let input = ExecutionInput::resolve(
                    target.clone(),
                    Some(ExecutionPeriod::resolve(Some(&january))?),
                )?;
                let first = context.executions(&input)?;
                let second = context.executions(&input.clone())?;
                assert!(Arc::ptr_eq(&first, &second));
                assert_eq!(first.evidence.executions, 3);
                assert_eq!(first.mean().value().unwrap().as_str(), "10");
                let february = Period {
                    from: january.until,
                    until: Some("2026-03-01T00:00:00Z".parse().unwrap()),
                };
                let other_period = ExecutionInput::resolve(
                    target,
                    Some(ExecutionPeriod::resolve(Some(&february))?),
                )?;
                let later = context.executions(&other_period)?;
                assert!(!Arc::ptr_eq(&first, &later));
                assert_eq!(later.evidence.executions, 0);
                let other_target = Arc::new(ResolvedTarget::Selection {
                    collection_id: "gift".into(),
                    criteria: market_gifts::GiftCriteria {
                        models: vec!["otherm".into()],
                        ..Default::default()
                    },
                });
                let other_selection = ExecutionInput::resolve(
                    other_target,
                    Some(ExecutionPeriod::resolve(Some(&january))?),
                )?;
                let other = context.executions(&other_selection)?;
                assert_eq!(other.evidence.executions, 0);
                assert!(
                    Arc::ptr_eq(&first, &context.executions(&input)?),
                    "alternating batch inputs reuse their earlier scan"
                );
                Ok(())
            })
            .await
            .unwrap()
            .unwrap();
        reads.shutdown().await;
    }
}
