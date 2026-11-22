//! Reusable evaluated reads: dependencies and deadlines stay internal to the service.
use super::read_set::ReadSet;
use super::*;
use crate::{floors::BookCache, source::ValuationSnapshot};
use std::sync::Arc;

pub(crate) struct ValuationRead {
    pub report: Arc<ValuationReport>,
    pub(super) reads: ReadSet,
    pub(super) external_gift: Option<market_gifts::TelegramGiftDetails>,
}
impl ValuationRead {
    #[cfg(test)]
    pub(crate) fn statistics(&self) -> &[Arc<crate::floors::Statistics>] {
        &self.reads.statistics
    }

    pub fn deadline(&self) -> tokio::time::Instant {
        self.reads.deadline
    }
    pub fn read(
        snapshot: &ValuationSnapshot,
        input: &PreparedQuery,
        books: &BookCache,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, ValuationError> {
        let stale = snapshot.sync()?.state != market_gifts::SyncState::Live;
        if let Some(previous) =
            previous.filter(|previous| previous.external_gift == input.external_gift)
        {
            let due = tokio::time::Instant::now() >= previous.deadline();
            if !(due && transient(&previous.report.results)) && previous.reads.current(snapshot)? {
                if !due && previous.report.stale == stale {
                    return Ok(previous.clone());
                }
                let mut reads = previous.reads.clone();
                if due {
                    reads.renew();
                }
                let report = if previous.report.stale == stale {
                    previous.report.clone()
                } else {
                    let mut report = previous.report.as_ref().clone();
                    report.stale = stale;
                    Arc::new(report)
                };
                return Ok(Arc::new(Self {
                    report,
                    reads,
                    external_gift: previous.external_gift.clone(),
                }));
            }
        }
        snapshot.require_currency(&input.currency_id)?;
        let target = Arc::new(if let Some(gift) = &input.external_gift {
            ResolvedTarget::Gift {
                gift: snapshot.telegram_gift(gift)?,
                slug: Some(gift.slug.clone()),
            }
        } else {
            input.target.resolve(snapshot)?
        });
        let collections = match target.as_ref() {
            ResolvedTarget::Catalog(selection) => selection.collection_ids.clone(),
            target => vec![target.collection().expect("single selection").clone()],
        };
        let context = ValuationContext::new(snapshot, &input.currency_id, books);
        let mut results = Vec::with_capacity(input.strategies.len());
        for entry in &input.strategies {
            snapshot.check_cancelled()?;
            results.push(
                match entry.evaluate(target.clone(), input.period, &context) {
                    Ok(result) => StrategyOutcome::Value(result),
                    Err(error) => StrategyOutcome::Error {
                        strategy: StrategyId::new(entry.metadata().id)
                            .expect("registered strategy ID"),
                        error: error.into(),
                    },
                },
            );
        }
        snapshot.check_cancelled()?;
        let mut reads = context.reads.into_inner();
        reads.finish(
            snapshot,
            collections,
            &input.currency_id,
            match &input.target {
                ValuationTarget::Gift { gift } => Some(gift),
                _ => None,
            },
        )?;
        reads.deadline = retry_deadline(&results, reads.deadline);
        let deadline = reads.deadline;
        if deadline <= tokio::time::Instant::now() {
            return Err(ValuationError::unavailable(
                "Valuation refresh elapsed during the read; retry",
            ));
        }
        let stale = snapshot.sync()?.state != market_gifts::SyncState::Live;
        Ok(Arc::new(Self {
            external_gift: input.external_gift.clone(),
            report: Arc::new(ValuationReport {
                target: target.as_ref().clone(),
                currency_id: input.currency_id.clone(),
                evaluated_at: jiff::Timestamp::now(),
                stale,
                results,
            }),
            reads,
        }))
    }
}

// Missing samples and invalid baselines are domain facts; only transient service failures retry.
fn retry_deadline(
    results: &[StrategyOutcome],
    deadline: tokio::time::Instant,
) -> tokio::time::Instant {
    if transient(results) {
        deadline.min(tokio::time::Instant::now() + std::time::Duration::from_secs(5))
    } else {
        deadline
    }
}
fn transient(results: &[StrategyOutcome]) -> bool {
    results.iter().any(|result| {
        matches!(result, StrategyOutcome::Error {error,..}
        if matches!(error.code, StrategyFailureCode::Unavailable | StrategyFailureCode::TimedOut))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn temporary_failures_retry_without_extending_expiry_or_polling_sparse_markets() {
        let now = tokio::time::Instant::now();
        let normal = now + std::time::Duration::from_secs(300);
        for code in [
            StrategyFailureCode::Unavailable,
            StrategyFailureCode::TimedOut,
        ] {
            let outcomes = [StrategyOutcome::Error {
                strategy: StrategyId::new("ask_floor").unwrap(),
                error: StrategyFailure {
                    code,
                    message: "Try later".into(),
                },
            }];
            assert_eq!(
                retry_deadline(&outcomes, normal),
                now + std::time::Duration::from_secs(5)
            );
            let expiry = now + std::time::Duration::from_secs(1);
            assert_eq!(retry_deadline(&outcomes, expiry), expiry);
        }
        for code in [
            StrategyFailureCode::NotEnoughData,
            StrategyFailureCode::InvalidBaseline,
            StrategyFailureCode::CapacityExceeded,
            StrategyFailureCode::UnsupportedTarget,
        ] {
            let outcomes = [StrategyOutcome::Error {
                strategy: StrategyId::new("ask_floor").unwrap(),
                error: StrategyFailure {
                    code,
                    message: "Unavailable calculation".into(),
                },
            }];
            assert_eq!(retry_deadline(&outcomes, normal), normal);
        }
    }
}
