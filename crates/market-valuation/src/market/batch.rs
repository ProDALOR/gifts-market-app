//! Ordered independent results over one admitted snapshot and collection-scoped contexts.
use super::*;
use crate::{floors::BookCache, source::ValuationSnapshot};
use std::{collections::BTreeMap, sync::Arc};

pub const MAX_VALUATION_BATCH: usize = 100;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationBatchQuery {
    /// Results have the same order, including repeated inputs and individual failures.
    #[cfg_attr(feature = "schema", schema(min_items = 1, max_items = 100))]
    pub queries: Vec<ValuationQuery>,
}
impl ValuationBatchQuery {
    pub fn validate(&self) -> Result<(), ValuationError> {
        if self.queries.is_empty() || self.queries.len() > MAX_VALUATION_BATCH {
            return Err(ValuationError::invalid("select 1..=100 batch queries"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationBatchItem {
    Value { data: Arc<ValuationReport> },
    Error { error: StrategyFailure },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationBatchReport {
    pub results: Vec<ValuationBatchItem>,
}

pub(crate) struct BatchInput {
    pub target: Arc<ResolvedTarget>,
    pub currency: CurrencyId,
    pub strategies: Arc<[&'static crate::strategy::Registration]>,
    pub period: Option<ExecutionPeriod>,
}
impl BatchInput {
    pub(crate) fn resolve(
        snapshot: &ValuationSnapshot,
        input: PreparedQuery,
        resolution_failure: Option<ValuationError>,
    ) -> Result<Self, ValuationError> {
        let target = input.resolve_target(snapshot).map_err(|error| {
            if error.code() == ValuationErrorCode::NotFound {
                resolution_failure.unwrap_or(error)
            } else {
                error
            }
        })?;
        Ok(Self {
            target: Arc::new(target),
            currency: input.currency_id,
            strategies: input.strategies.into(),
            period: input.period,
        })
    }
}
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    Collection(market_gifts::CollectionId),
    Catalog(usize),
}
/// Shared by finite API batches and inventory valuation. The projection keeps
/// inventory totals from retaining thousands of full strategy reports.
pub(crate) fn evaluate_batch<T>(
    snapshot: &ValuationSnapshot,
    books: &BookCache,
    inputs: Vec<Result<BatchInput, StrategyFailure>>,
    mut project: impl FnMut(&BatchInput, Vec<StrategyOutcome>) -> T,
    mut progress: impl FnMut(usize),
) -> Result<Vec<Result<T, StrategyFailure>>, ValuationError> {
    let count = inputs.len();
    let mut results: Vec<_> = (0..count).map(|_| None).collect();
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    let mut completed = 0;
    for (index, input) in inputs.into_iter().enumerate() {
        snapshot.check_cancelled()?;
        match input {
            Ok(input) => {
                let scope = input
                    .target
                    .collection()
                    .cloned()
                    .map(Scope::Collection)
                    .unwrap_or(Scope::Catalog(index));
                groups
                    .entry((input.currency.clone(), scope))
                    .or_default()
                    .push((index, input));
            }
            Err(error) => {
                results[index] = Some(Err(error));
                completed += 1;
            }
        }
    }
    progress(completed);
    for ((currency, _), entries) in groups {
        let context = ValuationContext::new(snapshot, &currency, books);
        for (index, input) in entries {
            snapshot.check_cancelled()?;
            let outcomes = evaluate_strategies(
                input.target.clone(),
                &input.strategies,
                input.period,
                &context,
            )?;
            results[index] = Some(Ok(project(&input, outcomes)));
            completed += 1;
            if completed.is_multiple_of(25) {
                progress(completed);
            }
        }
        progress(completed);
    }
    snapshot.check_cancelled()?;
    Ok(results
        .into_iter()
        .map(|value| value.expect("every batch input produces a result"))
        .collect())
}

pub(super) fn evaluate_strategies(
    target: Arc<ResolvedTarget>,
    strategies: &[&'static crate::strategy::Registration],
    period: Option<ExecutionPeriod>,
    context: &ValuationContext<'_>,
) -> Result<Vec<StrategyOutcome>, ValuationError> {
    let mut results = Vec::with_capacity(strategies.len());
    for entry in strategies {
        context.snapshot().check_cancelled()?;
        results.push(match entry.evaluate(target.clone(), period, context) {
            Ok(value) => StrategyOutcome::Value(value),
            Err(error) => StrategyOutcome::Error {
                strategy: StrategyId::new(entry.metadata().id).expect("registered strategy ID"),
                error: error.into(),
            },
        });
    }
    context.snapshot().check_cancelled()?;
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn batches_preserve_input_order_errors_and_share_preparations_with_single_queries() {
        let (_dir, writer) = crate::gift_floor::tests::book();
        let data = crate::test_support::MarketDataService::start(writer.reader(), 2, 1)
            .await
            .unwrap();
        let (handle, task) = crate::ValuationHandle::start(&data.arbiter(), data.reader.executor())
            .await
            .unwrap();
        let gift = |number| ValuationQuery {
            target: ValuationTarget::GiftAttributes(GiftAttributes {
                collection_id: "gift".into(),
                traits: market_gifts::GiftTraits {
                    model: Some("m".into()),
                    backdrop: Some("b".into()),
                    pattern: None,
                },
                number: Some(market_gifts::GiftNumber::new(number).unwrap()),
            }),
            currency_id: "GRAM".into(),
            strategies: vec![StrategyId::new("gift_floor").unwrap()],
            period: None,
        };
        let mut invalid = gift(2);
        invalid.strategies = vec![StrategyId::new("unknown").unwrap()];
        let mut missing = gift(3);
        missing.target = ValuationTarget::Selection {
            collection_id: "missing".into(),
            criteria: Default::default(),
        };
        assert!(
            handle
                .batch(ValuationBatchQuery { queries: vec![] })
                .await
                .is_err()
        );
        assert!(
            handle
                .batch(ValuationBatchQuery {
                    queries: vec![gift(1); 101]
                })
                .await
                .is_err()
        );
        let batch = handle
            .batch(ValuationBatchQuery {
                queries: vec![gift(9), invalid, missing, gift(1), gift(9)],
            })
            .await
            .unwrap();
        assert_eq!(batch.results.len(), 5);
        match (&batch.results[0], &batch.results[4]) {
            (
                ValuationBatchItem::Value { data: first },
                ValuationBatchItem::Value { data: last },
            ) => {
                assert!(
                    Arc::ptr_eq(first, last),
                    "duplicates share immutable reports"
                );
            }
            _ => panic!("expected duplicate values"),
        }
        for index in [0, 3, 4] {
            let ValuationBatchItem::Value { data: report } = &batch.results[index] else {
                panic!("expected a valuation")
            };
            let single = handle
                .query(gift(if index == 3 { 1 } else { 9 }))
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&report.target).unwrap(),
                serde_json::to_value(&single.target).unwrap()
            );
            assert_eq!(
                serde_json::to_value(&report.results).unwrap(),
                serde_json::to_value(&single.results).unwrap()
            );
        }
        for index in [1, 2] {
            assert!(
                matches!(&batch.results[index], ValuationBatchItem::Error { error } if error.code == StrategyFailureCode::InvalidQuery)
            );
        }
        assert_eq!(
            handle
                .books
                .preparations
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        let held = crate::integration_tests::hold(&handle.source).await;
        let mut invalid = gift(1);
        invalid.strategies = vec![StrategyId::new("unknown").unwrap()];
        let errors = tokio::time::timeout(
            std::time::Duration::from_millis(300),
            handle.batch(ValuationBatchQuery {
                queries: vec![invalid; 100],
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            errors
                .results
                .iter()
                .all(|item| matches!(item, ValuationBatchItem::Error { .. })),
            "invalid queries need no read admission"
        );
        drop(held);
        task.shutdown().await.unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
    #[tokio::test]
    async fn cancelling_a_batch_stops_before_projecting_the_remaining_inputs() {
        let (_dir, writer) = crate::gift_floor::tests::book();
        let stop = tokio_util::sync::CancellationToken::new();
        let reads = market_replica::ReadRuntime::new(writer.reader(), 2, stop.clone()).unwrap();
        let source = crate::ValuationSource::new(reads.handle());
        let cancel = stop.clone();
        let projected = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = projected.clone();
        let result = source
            .prepare(move |snapshot| {
                let books = BookCache::default();
                let query = ValuationQuery {
                    target: ValuationTarget::Selection {
                        collection_id: "gift".into(),
                        criteria: Default::default(),
                    },
                    currency_id: "GRAM".into(),
                    strategies: vec![StrategyId::new("ask_floor").unwrap()],
                    period: None,
                };
                let inputs = (0..100)
                    .map(|_| {
                        BatchInput::resolve(&snapshot, query.clone().prepare().unwrap(), None)
                            .map_err(Into::into)
                    })
                    .collect();
                evaluate_batch(
                    &snapshot,
                    &books,
                    inputs,
                    |_, _| {
                        seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        cancel.cancel();
                    },
                    |_| {},
                )
            })
            .await;
        assert!(result.is_err() || result.unwrap().is_err());
        assert_eq!(projected.load(std::sync::atomic::Ordering::Relaxed), 1);
        reads.shutdown().await;
    }
}
