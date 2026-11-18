//! Floor actors own a small bounded portfolio cache and evaluate on demand.
use crate::{
    Evaluate, Lifecycle, Mark,
    calculation::{self, FloorKind},
};
use actix::{Actor, ActorFutureExt, AtomicResponse, Context, Handler, WrapFuture};
use market_actors::{
    OrderbookReader, ReadDependencies,
    error::{DataError, DataErrorCode},
};
use market_gifts::valuation::*;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
const CACHE_LIMIT: usize = 16;
const FRESHNESS: Duration = Duration::from_secs(30);
pub struct FloorActor {
    source: OrderbookReader,
    life: Lifecycle,
    kind: FloorKind,
    cache: Vec<CachedPortfolio>,
}
struct CachedPortfolio {
    input: PortfolioPositions,
    dependencies: Arc<ReadDependencies>,
    prepared_at: Instant,
    used: Instant,
    mark: Arc<Mark<Valuation>>,
}
impl FloorActor {
    pub fn model(source: OrderbookReader, life: Lifecycle) -> Self {
        Self {
            source,
            life,
            kind: FloorKind::Model,
            cache: Vec::new(),
        }
    }
    pub fn collection(source: OrderbookReader, life: Lifecycle) -> Self {
        Self {
            source,
            life,
            kind: FloorKind::Collection,
            cache: Vec::new(),
        }
    }
}
impl Actor for FloorActor {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.life.started(ctx);
    }
}
impl Handler<Evaluate<PortfolioPositions, Valuation, DataError>> for FloorActor {
    type Result = AtomicResponse<Self, Result<Arc<Mark<Valuation>>, DataError>>;
    fn handle(
        &mut self,
        message: Evaluate<PortfolioPositions, Valuation, DataError>,
        _: &mut Self::Context,
    ) -> Self::Result {
        let (input, request) = message.into_parts();
        let input = calculation::normalize(input);
        let source = self.source.clone();
        let cached = input.as_ref().ok().and_then(|input| {
            self.cache
                .iter_mut()
                .find(|entry| {
                    entry.input == *input
                        && entry.prepared_at.elapsed() < FRESHNESS
                        && source.is_current(&entry.dependencies)
                })
                .map(|entry| {
                    entry.used = Instant::now();
                    entry.mark.clone()
                })
        });
        let kind = self.kind;
        let stop = self.life.stop_token();
        let work = async move {
            let _request = request;
            let input = input?;
            if let Some(mark) = cached {
                return Ok((input, mark, None));
            }
            let positions = input.clone();
            let (prepared, dependencies) = tokio::select! {biased;
                _=stop.cancelled()=>return Err(DataError::unavailable("Valuation actor is stopping")),
                _=_request.cancelled.cancelled()=>return Err(DataError::unavailable("Valuation request cancelled")),
                result=source.prepare(move|snapshot|calculation::portfolio(snapshot,positions,kind))=>result?,
            };
            let mark = Arc::new(Mark::evaluated(kind.id(), prepared));
            validate(&mark)?;
            Ok((input, mark, Some(dependencies)))
        };
        AtomicResponse::new(Box::pin(work.into_actor(self).map(|result, actor, _| {
            result.map(|(input, mark, dependencies)| {
                if let Some(dependencies) = dependencies
                    && actor.source.is_current(&dependencies)
                {
                    actor.cache.retain(|entry| {
                        entry.input != input && entry.prepared_at.elapsed() < FRESHNESS
                    });
                    if actor.cache.len() >= CACHE_LIMIT {
                        let oldest = actor
                            .cache
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, e)| e.used)
                            .map(|(i, _)| i)
                            .unwrap();
                        actor.cache.swap_remove(oldest);
                    }
                    actor.cache.push(CachedPortfolio {
                        input,
                        dependencies: Arc::new(dependencies),
                        prepared_at: Instant::now(),
                        used: Instant::now(),
                        mark: mark.clone(),
                    });
                }
                mark
            })
        })))
    }
}
fn validate(mark: &Mark<Valuation>) -> Result<(), DataError> {
    if mark.priced == 0 && mark.unpriced > 0 {
        return Err(DataError::valuation(
            DataErrorCode::NotEnoughData,
            "No requested positions could be valued with this strategy and currency",
            ValuationErrorDetails {
                strategy: StrategyId::new(mark.strategy)
                    .map_err(|e| DataError::invalid(e.to_string()))?,
                currency_id: Some(mark.currency_id.clone()),
                revision: Some(mark.revision.clone()),
                required_samples: Some(1),
                available_samples: Some(0),
                context: Some(ValuationErrorContext::Portfolio {
                    unpriced: mark.rows.clone(),
                }),
            },
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[actix::test]
    async fn cache_hits_do_not_extend_preparation_freshness() {
        let (_dir, writer) = crate::integration_tests::model_fixture();
        let data = market_actors::MarketDataService::start(writer.reader(), 2, 1)
            .await
            .unwrap();
        let input = PortfolioPositions {
            positions: vec![],
            currency_id: "GRAM".into(),
        };
        let (prepared, dependencies) = calculation::portfolio(
            data.orderbooks.snapshot().unwrap(),
            input.clone(),
            FloorKind::Model,
        )
        .unwrap();
        let mark = Arc::new(Mark::evaluated("model_floor", prepared));
        let service = crate::Service::start("model_floor", Default::default(), |life| {
            let mut actor = FloorActor::model(data.orderbooks.clone(), life);
            actor.cache.push(CachedPortfolio {
                input: input.clone(),
                dependencies: Arc::new(dependencies),
                prepared_at: Instant::now() - Duration::from_millis(29_500),
                used: Instant::now(),
                mark: mark.clone(),
            });
            actor
        })
        .unwrap();
        assert!(Arc::ptr_eq(
            &mark,
            &service.handle.evaluate(input.clone()).await.unwrap()
        ));
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(!Arc::ptr_eq(
            &mark,
            &service.handle.evaluate(input).await.unwrap()
        ));
        service.task.shutdown().await.unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
    #[test]
    fn normalize_groups_once_and_enforces_domain_and_service_limits() {
        let position = ModelPosition {
            model: ModelLookup::named("Gift", "Model").unwrap(),
            quantity: 1,
        };
        let input = |positions| PortfolioPositions {
            positions,
            currency_id: "GRAM".into(),
        };
        assert_eq!(
            calculation::normalize(input(vec![position.clone(); 3]))
                .unwrap()
                .positions[0]
                .quantity,
            3
        );
        assert!(calculation::normalize(input(vec![position.clone(); 1001])).is_err());
        assert!(
            calculation::normalize(input(vec![ModelPosition {
                quantity: 10001,
                ..position.clone()
            }]))
            .is_err()
        );
        assert!(
            calculation::normalize(input(vec![ModelPosition {
                quantity: 0,
                ..position
            }]))
            .is_err()
        );
    }
}
