//! Reusable evaluated reads: dependencies and deadlines stay internal to the service.
use super::*;
use crate::{
    floors::{BookCache, Statistics},
    source::{BookScope, OrderDependency, ReadDependencies, ValuationSnapshot},
};
use std::sync::Arc;

pub(crate) struct ValuationRead {
    pub report: Arc<ValuationReport>,
    dependencies: ReadDependencies,
    pub(crate) statistics: Vec<Arc<Statistics>>,
    observations: Vec<OrderDependency>,
    pub deadline: tokio::time::Instant,
}
impl ValuationRead {
    pub fn read(
        snapshot: &ValuationSnapshot,
        input: &PreparedQuery,
        books: &BookCache,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, ValuationError> {
        let stale = snapshot.sync()?.state != market_gifts::SyncState::Live;
        if let Some(previous) = previous {
            let mut current = tokio::time::Instant::now() < previous.deadline
                && snapshot.dependencies_data_current(&previous.dependencies)?;
            for stats in &previous.statistics {
                if !current {
                    break;
                }
                current = stats.current(snapshot)?;
            }
            for observation in &previous.observations {
                if !current {
                    break;
                }
                current = observation.current(snapshot)?;
            }
            if current {
                if previous.report.stale == stale {
                    return Ok(previous.clone());
                }
                let mut report = previous.report.as_ref().clone();
                report.stale = stale;
                return Ok(Arc::new(Self {
                    report: Arc::new(report),
                    dependencies: previous.dependencies.clone(),
                    statistics: previous.statistics.clone(),
                    observations: previous.observations.clone(),
                    deadline: previous.deadline,
                }));
            }
        }
        snapshot.require_currency(&input.currency_id)?;
        let target = Arc::new(input.target.resolve(snapshot)?);
        let collections = match target.as_ref() {
            ResolvedTarget::Catalog(selection) => selection.collection_ids.clone(),
            target => vec![target.collection().expect("single selection").clone()],
        };
        let dependencies = snapshot.target_dependencies(
            collections.into_iter().map(|collection| BookScope {
                collection,
                currency: input.currency_id.clone(),
            }),
            &input.currency_id,
            match &input.target {
                ValuationTarget::Gift { gift } => Some(gift),
                _ => None,
            },
        )?;
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
        let deadline = context.deadline.get();
        if deadline <= tokio::time::Instant::now() {
            return Err(ValuationError::unavailable(
                "Asks expired during the read; retry",
            ));
        }
        let statistics = context.statistics.into_inner();
        let observations = context.observations.into_inner();
        let stale = snapshot.sync()?.state != market_gifts::SyncState::Live;
        Ok(Arc::new(Self {
            report: Arc::new(ValuationReport {
                target: target.as_ref().clone(),
                currency_id: input.currency_id.clone(),
                evaluated_at: jiff::Timestamp::now(),
                stale,
                results,
            }),
            dependencies,
            statistics,
            observations,
            deadline,
        }))
    }
}
