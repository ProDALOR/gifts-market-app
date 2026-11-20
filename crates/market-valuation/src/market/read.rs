//! Reusable evaluated reads: dependencies and deadlines stay internal to the service.
use super::read_set::ReadSet;
use super::*;
use crate::{floors::BookCache, source::ValuationSnapshot};
use std::sync::Arc;

pub(crate) struct ValuationRead {
    pub report: Arc<ValuationReport>,
    pub(super) reads: ReadSet,
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
        if let Some(previous) = previous {
            if previous.reads.current(snapshot)? {
                if previous.report.stale == stale {
                    return Ok(previous.clone());
                }
                let mut report = previous.report.as_ref().clone();
                report.stale = stale;
                return Ok(Arc::new(Self {
                    report: Arc::new(report),
                    reads: previous.reads.clone(),
                }));
            }
        }
        snapshot.require_currency(&input.currency_id)?;
        let target = Arc::new(input.target.resolve(snapshot)?);
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
                        error: StrategyFailure {
                            code: error.code(),
                            message: error.message().to_owned(),
                        },
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
        let deadline = reads.deadline;
        if deadline <= tokio::time::Instant::now() {
            return Err(ValuationError::unavailable(
                "Asks expired during the read; retry",
            ));
        }
        let stale = snapshot.sync()?.state != market_gifts::SyncState::Live;
        Ok(Arc::new(Self {
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
