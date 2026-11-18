//! One supervised gift strategy. Cached collection statistics are checked against each pinned view.
use crate::gift_calculation::{REFRESH, Statistics};
use crate::source::{BookScope, ValuationSnapshot, ValuationSource};
use crate::*;
use crate::{
    Evaluate, Handle, Lifecycle, Mark, Prepared, Service, ServiceError, Task, ValuationError,
};
use actix::{Actor, ActorFutureExt, AsyncContext, Context, Handler, ResponseActFuture, WrapFuture};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

pub const GIFT_STRATEGY: &str = "gift_floor";
pub type GiftValuationResult = Mark<GiftValuation>;
type NativeHandle = Handle<GiftValuationRequest, GiftValuation, ValuationError>;
#[derive(Clone)]
pub struct ValuationHandle(NativeHandle);
impl From<NativeHandle> for ValuationHandle {
    fn from(handle: NativeHandle) -> Self {
        Self(handle)
    }
}
impl ValuationHandle {
    pub async fn gift(
        &self,
        request: GiftValuationRequest,
    ) -> Result<Arc<GiftValuationResult>, ValuationError> {
        let currency = request.currency_id.clone();
        crate::client::invoke(
            &self.0,
            request,
            StrategyId::new(GIFT_STRATEGY).unwrap(),
            currency,
        )
        .await
    }
    pub async fn start(
        arbiter: &actix::ArbiterHandle,
        reads: market_replica::ReadExecutor,
    ) -> Result<(Self, Task), ServiceError> {
        let service = Service::start_on(
            arbiter,
            GIFT_STRATEGY,
            move |life| GiftValuationActor::new(ValuationSource::new(reads), life),
            crate::Config {
                max_requests: 32,
                ..Default::default()
            },
        )
        .await?;
        Ok((service.handle.into(), service.task))
    }
}
struct CachedBook {
    stats: Arc<Statistics>,
    used: Instant,
}
#[derive(Default)]
struct Books(HashMap<BookScope, CachedBook>);
impl Books {
    fn get(
        &mut self,
        snapshot: &ValuationSnapshot,
        scope: &BookScope,
    ) -> Result<Arc<Statistics>, ValuationError> {
        let now = Instant::now();
        self.0
            .retain(|_, entry| entry.used.elapsed() < Duration::from_secs(300));
        if let Some(cached) = self.0.get_mut(scope) {
            if cached.stats.current(snapshot)? {
                cached.used = now;
                return Ok(cached.stats.clone());
            }
            self.0.remove(scope);
        }
        let stats = Arc::new(Statistics::read(snapshot, scope)?);
        snapshot.check_cancelled()?;
        if self.0.len() >= 8 {
            let oldest = self
                .0
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(scope, _)| scope.clone())
                .unwrap();
            self.0.remove(&oldest);
        }
        self.0.insert(
            scope.clone(),
            CachedBook {
                stats: stats.clone(),
                used: now,
            },
        );
        Ok(stats)
    }
}
pub struct GiftValuationActor {
    life: Lifecycle,
    source: ValuationSource,
    // Only tracked read workers access this cache. The Arbiter never takes its lock.
    books: Arc<Mutex<Books>>,
    active: Option<(BookScope, Instant)>,
    refreshing: bool,
}
impl GiftValuationActor {
    pub fn new(source: ValuationSource, life: Lifecycle) -> Self {
        Self {
            life,
            source,
            books: Arc::new(Mutex::new(Books::default())),
            active: None,
            refreshing: false,
        }
    }
}
impl Actor for GiftValuationActor {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.life.started(ctx);
        ctx.run_interval(REFRESH, |actor, ctx| {
            let Some((scope, used)) = actor.active.as_ref() else {
                return;
            };
            if actor.refreshing || used.elapsed() >= Duration::from_secs(300) {
                return;
            }
            actor.refreshing = true;
            let source = actor.source.clone();
            let books = actor.books.clone();
            let scope = scope.clone();
            ctx.spawn(
                async move {
                    source
                        .prepare(move |snapshot| {
                            books
                                .lock()
                                .map_err(|_| {
                                    ValuationError::unavailable("Valuation cache is unavailable")
                                })?
                                .get(&snapshot, &scope)
                        })
                        .await
                }
                .into_actor(actor)
                .map(|_, actor, _| actor.refreshing = false),
            );
        });
    }
}
impl Handler<Evaluate<GiftValuationRequest, GiftValuation, ValuationError>> for GiftValuationActor {
    type Result = ResponseActFuture<Self, Result<Arc<GiftValuationResult>, ValuationError>>;
    fn handle(
        &mut self,
        message: Evaluate<GiftValuationRequest, GiftValuation, ValuationError>,
        _: &mut Self::Context,
    ) -> Self::Result {
        let (input, request) = message.into_parts();
        let source = self.source.clone();
        let books = self.books.clone();
        let stop = self.life.stop_token();
        Box::pin(async move {
            let cancelled = request.cancelled.clone();
            let _request = request;
            let work = source.prepare(move |snapshot| -> Result<_, ValuationError> {
                snapshot.require_currency(&input.currency_id)?;
                let gift = snapshot.gift(&input.gift)?;
                let scope = BookScope { collection: gift.collection_id.clone(), currency: input.currency_id.clone() };
                let stats = books.lock().map_err(|_| ValuationError::unavailable("Valuation cache is unavailable"))?.get(&snapshot, &scope)?;
                let data = stats.report(gift, input.gift, input.currency_id, snapshot.sync()?);
                snapshot.check_cancelled()?;
                Ok((Prepared::new(data, snapshot.revision()?), scope))
            });
            tokio::select! {biased;
                _=stop.cancelled()=>Err(ValuationError::unavailable("Valuation actor is stopping")),
                _=cancelled.cancelled()=>Err(ValuationError::unavailable("Valuation request cancelled")),
                result=tokio::time::timeout(Duration::from_secs(30),work)=> {
                    result.map_err(|_|ValuationError::timed_out("Gift valuation timed out"))??
                }
            }
        }.into_actor(self).map(|result, actor, _| result.map(|(prepared, scope)| {
            actor.active = Some((scope, Instant::now()));
            Arc::new(Mark::evaluated(GIFT_STRATEGY, prepared))
        })))
    }
}

#[cfg(test)]
#[path = "gift_tests.rs"]
mod tests;
