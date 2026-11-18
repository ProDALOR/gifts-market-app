//! Floor actors own a small bounded portfolio cache and evaluate on demand.
use crate::{
    Evaluate, Lifecycle, Mark,
    calculation::{self, FloorKind},
};
use crate::{ValuationError, ValuationErrorCode};
use actix::{Actor, ActorFutureExt, AtomicResponse, Context, Handler, WrapFuture};
use market_actors::{MarketReader, ReadDependencies};
use market_gifts::valuation::*;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
const CACHE_LIMIT: usize = 16;
const FRESHNESS: Duration = Duration::from_secs(30);
pub struct FloorActor {
    source: MarketReader,
    life: Lifecycle,
    kind: FloorKind,
    cache: Vec<CachedPortfolio>,
}
struct CachedPortfolio {
    input: PortfolioPositions,
    dependencies: ReadDependencies,
    prepared_at: Instant,
    used: Instant,
    mark: Arc<Mark<Valuation>>,
}
impl FloorActor {
    pub fn model(source: MarketReader, life: Lifecycle) -> Self {
        Self {
            source,
            life,
            kind: FloorKind::Model,
            cache: Vec::new(),
        }
    }
    pub fn collection(source: MarketReader, life: Lifecycle) -> Self {
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
impl Handler<Evaluate<PortfolioPositions, Valuation, ValuationError>> for FloorActor {
    type Result = AtomicResponse<Self, Result<Arc<Mark<Valuation>>, ValuationError>>;
    fn handle(
        &mut self,
        message: Evaluate<PortfolioPositions, Valuation, ValuationError>,
        _: &mut Self::Context,
    ) -> Self::Result {
        let (input, request) = message.into_parts();
        let input = calculation::normalize(input);
        let source = self.source.clone();
        let cached = input.as_ref().ok().and_then(|input| {
            self.cache
                .iter_mut()
                .find(|entry| entry.input == *input && entry.prepared_at.elapsed() < FRESHNESS)
                .map(|entry| {
                    entry.used = Instant::now();
                    (
                        entry.mark.clone(),
                        entry.dependencies.clone(),
                        entry.prepared_at,
                    )
                })
        });
        let kind = self.kind;
        let stop = self.life.stop_token();
        let work = async move {
            let cancelled = request.cancelled.clone();
            let _request = request;
            let evaluate = async move {
                let input = input?;
                if let Some((mark, dependencies, prepared_at)) = cached
                    && source.is_current(&dependencies).await
                    && prepared_at.elapsed() < FRESHNESS
                {
                    return Ok((input, mark, None, prepared_at));
                }
                let positions = input.clone();
                let ((prepared, dependencies), prepared_at) = source
                    .prepare(move |snapshot| {
                        calculation::portfolio(snapshot, positions, kind)
                            .map(|result| (result, Instant::now()))
                    })
                    .await??;
                let mark = Arc::new(Mark::evaluated(kind.id(), prepared));
                validate(&mark)?;
                let dependencies = if source.is_current(&dependencies).await
                    && prepared_at.elapsed() < FRESHNESS
                {
                    Some(dependencies)
                } else {
                    None
                };
                Ok((input, mark, dependencies, prepared_at))
            };
            tokio::select! {biased;
                _=stop.cancelled()=>Err(ValuationError::unavailable("Valuation actor is stopping")),
                _=cancelled.cancelled()=>Err(ValuationError::unavailable("Valuation request cancelled")),
                result=evaluate=>result,
            }
        };
        AtomicResponse::new(Box::pin(work.into_actor(self).map(|result, actor, _| {
            result.map(|(input, mark, dependencies, prepared_at)| {
                if let Some(dependencies) = dependencies {
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
                        dependencies,
                        prepared_at,
                        used: Instant::now(),
                        mark: mark.clone(),
                    });
                }
                mark
            })
        })))
    }
}
fn validate(mark: &Mark<Valuation>) -> Result<(), ValuationError> {
    if mark.priced == 0 && mark.unpriced > 0 {
        return Err(ValuationError::with_details(
            ValuationErrorCode::NotEnoughData,
            "No requested positions could be valued with this strategy and currency",
            ValuationErrorDetails {
                strategy: StrategyId::new(mark.strategy)
                    .map_err(|e| ValuationError::invalid(e.to_string()))?,
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
            data.reader.snapshot().unwrap(),
            input.clone(),
            FloorKind::Model,
        )
        .unwrap();
        let mark = Arc::new(Mark::evaluated("model_floor", prepared));
        let service = crate::Service::start("model_floor", Default::default(), |life| {
            let mut actor = FloorActor::model(data.reader.clone(), life);
            actor.cache.push(CachedPortfolio {
                input: input.clone(),
                dependencies,
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
    #[actix::test]
    async fn expiry_is_rechecked_after_waiting_for_metadata_validation() {
        use market_hub_protocol::wire as w;
        let (_dir, mut writer) = crate::integration_tests::model_fixture();
        let data = market_actors::MarketDataService::start(writer.reader(), 1, 1)
            .await
            .unwrap();
        let input = PortfolioPositions {
            positions: vec![],
            currency_id: "GRAM".into(),
        };
        let (prepared, dependencies) = calculation::portfolio(
            data.reader.snapshot().unwrap(),
            input.clone(),
            FloorKind::Model,
        )
        .unwrap();
        let original = Arc::new(Mark::evaluated("model_floor", prepared));
        writer
            .apply(w::Batch {
                cursor: "unrelated-metadata".into(),
                changes: vec![crate::integration_tests::entity(
                    w::entity::Entity::Currency(w::Currency {
                        id: "XTR".into(),
                        name: "Stars".into(),
                        decimals: 0,
                    }),
                )],
            })
            .unwrap();
        let held = crate::integration_tests::hold(&data.reader).await;
        let service = crate::Service::start("model_floor", Default::default(), |life| {
            let mut actor = FloorActor::model(data.reader.clone(), life);
            actor.cache.push(CachedPortfolio {
                input: input.clone(),
                dependencies,
                prepared_at: Instant::now() - Duration::from_millis(29_500),
                used: Instant::now(),
                mark: original.clone(),
            });
            actor
        })
        .unwrap();
        let mut reply = Box::pin(service.handle.evaluate(input));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut reply)
                .await
                .is_err()
        );
        tokio::time::sleep(Duration::from_millis(600)).await;
        drop(held);
        let fresh = tokio::time::timeout(Duration::from_secs(2), reply)
            .await
            .unwrap()
            .unwrap();
        assert!(!Arc::ptr_eq(&original, &fresh));
        assert_eq!(fresh.revision, "unrelated-metadata");
        service.task.shutdown().await.unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
}
