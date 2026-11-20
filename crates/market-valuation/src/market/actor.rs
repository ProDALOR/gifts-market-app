use super::*;
use crate::{floors::BookCache, source::ValuationSource};
use actix::{Actor, Context, Handler, ResponseFuture};
use std::{sync::Arc, time::Duration};
pub(crate) struct MarketValuationActor {
    source: ValuationSource,
    books: Arc<BookCache>,
    life: Lifecycle,
}
impl MarketValuationActor {
    pub fn new(source: ValuationSource, books: Arc<BookCache>, life: Lifecycle) -> Self {
        Self {
            source,
            books,
            life,
        }
    }
}
impl Actor for MarketValuationActor {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.life.started(ctx);
    }
}
impl Handler<Evaluate<crate::service::Work, crate::service::Output, ValuationError>>
    for MarketValuationActor
{
    type Result = ResponseFuture<Result<Arc<Mark<crate::service::Output>>, ValuationError>>;
    fn handle(
        &mut self,
        message: Evaluate<crate::service::Work, crate::service::Output, ValuationError>,
        _: &mut Self::Context,
    ) -> Self::Result {
        let (input, request) = message.into_parts();
        let source = self.source.clone();
        let books = self.books.clone();
        let stop = self.life.stop_token();
        let strategy = self.life.strategy();
        Box::pin(async move {
            let cancelled = request.cancelled.clone();
            let _request = request;
            let work = source.prepare(move |snapshot| -> Result<_, ValuationError> {
                let output = match input {
                    crate::service::Work::Floors { request, previous } => {
                        crate::service::Output::Floors(crate::floor_projection::FloorRead::read(
                            &snapshot,
                            &request,
                            &books,
                            previous.as_ref(),
                        )?)
                    }
                    crate::service::Work::Query(input) => {
                        snapshot.require_currency(&input.currency_id)?;
                        let target = Arc::new(input.target.resolve(&snapshot)?);
                        let context = ValuationContext::new(&snapshot, &input.currency_id, &books);
                        let mut results = Vec::with_capacity(input.strategies.len());
                        for entry in &input.strategies {
                            snapshot.check_cancelled()?;
                            let result = entry.evaluate(target.clone(), input.period, &context);
                            results.push(match result {
                                Ok(result) => StrategyOutcome::Value(result),
                                Err(error) => StrategyOutcome::Error {
                                    strategy: StrategyId::new(entry.metadata().id)
                                        .expect("registered strategy ID"),
                                    error: StrategyFailure {
                                        code: error.code(),
                                        message: error.message().to_owned(),
                                    },
                                },
                            });
                        }
                        snapshot.check_cancelled()?;
                        let evaluated_at = jiff::Timestamp::now();
                        let stale = snapshot.sync()?.state != market_gifts::SyncState::Live;
                        let report = ValuationReport {
                            target: target.as_ref().clone(),
                            currency_id: input.currency_id.clone(),
                            evaluated_at,
                            stale,
                            results,
                        };
                        crate::service::Output::Query(Arc::new(report))
                    }
                };
                let stale = snapshot.sync()?.state != market_gifts::SyncState::Live;
                Ok(Arc::new(Mark::evaluated(
                    strategy,
                    Prepared::new(output, snapshot.revision()?).with_stale(stale),
                )))
            });
            tokio::select! {biased;
                _=stop.cancelled()=>Err(ValuationError::unavailable("Valuation service is stopping")),
                _=cancelled.cancelled()=>Err(ValuationError::unavailable("Valuation request cancelled")),
                result=tokio::time::timeout(Duration::from_secs(30),work)=>result.map_err(|_|ValuationError::timed_out("Valuation report timed out"))??,
            }
        })
    }
}
