//! Read access and bounded shared preparation; target/options belong to typed inputs.
use super::{ExecutionInput, execution::ExecutionStatistics};
use crate::{
    ValuationError,
    floors::{BookCache, Statistics},
    source::{BookScope, ValuationSnapshot},
};
use market_gifts::{CollectionId, CurrencyId, GiftOrder, GiftOrderQuery, OrderSource};
use std::{cell::RefCell, sync::Arc};
type ExecutionCache = Option<(
    ExecutionInput,
    Result<Arc<ExecutionStatistics>, ValuationError>,
)>;
pub struct ValuationContext<'a> {
    snapshot: &'a ValuationSnapshot,
    currency: &'a CurrencyId,
    books: &'a BookCache,
    executions: RefCell<ExecutionCache>,
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
            executions: RefCell::new(None),
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
        let result = self.books.get(
            self.snapshot,
            &BookScope {
                collection: collection.clone(),
                currency: self.currency.clone(),
            },
        );
        match result {
            Ok(stats) => {
                self.reads.borrow_mut().statistics(stats.clone())?;
                Ok(stats)
            }
            Err(error) => {
                self.observe(&GiftOrderQuery::new(market_gifts::GiftFilters::new(
                    collection.clone(),
                    self.currency.clone(),
                )))?;
                Err(error)
            }
        }
    }

    /// Strategies read observations here so watches follow their actual sources and expiry.
    pub fn orders(
        &self,
        query: &GiftOrderQuery,
        continuation: String,
    ) -> Result<crate::source::Page<GiftOrder>, ValuationError> {
        self.observe(query)?;
        let page = self.snapshot.valuation_orders(query, continuation)?;
        if query.source == OrderSource::Asks {
            let now = jiff::Timestamp::now();
            for expiry in page
                .data
                .iter()
                .filter_map(|order| order.expires_at)
                .filter(|expiry| *expiry > now)
            {
                let remaining = std::time::Duration::from_millis(
                    expiry
                        .as_millisecond()
                        .saturating_sub(now.as_millisecond())
                        .max(0) as u64,
                );
                let mut reads = self.reads.borrow_mut();
                reads.deadline = reads.deadline.min(tokio::time::Instant::now() + remaining);
            }
        }
        Ok(page)
    }

    fn observe(&self, query: &GiftOrderQuery) -> Result<(), ValuationError> {
        self.reads.borrow_mut().observe(self.snapshot, query)
    }

    pub(super) fn executions(
        &self,
        input: &ExecutionInput,
    ) -> Result<Arc<ExecutionStatistics>, ValuationError> {
        if let Some((key, result)) = &*self.executions.borrow()
            && key == input
        {
            return result.clone();
        }
        // One cache slot is enough for a query's shared mean/median traversal. Keep the key
        // explicit so another selection or period can never reuse the wrong observations.
        let result = ExecutionStatistics::read(self, input).map(Arc::new);
        *self.executions.borrow_mut() = Some((input.clone(), result.clone()));
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
                Ok(())
            })
            .await
            .unwrap()
            .unwrap();
        reads.shutdown().await;
    }
}
