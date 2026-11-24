use super::*;
use crate::{floors::BookCache, source::ValuationSource};
use actix::{Actor, Context, Handler, ResponseFuture};
use std::{sync::Arc, time::Duration};
pub(crate) struct MarketValuationActor {
    source: ValuationSource,
    books: Arc<BookCache>,
    reads: Arc<super::read_cache::ReadCache>,
    life: Lifecycle,
    resolver: crate::service::GiftResolverState,
}
impl MarketValuationActor {
    pub fn new(
        source: ValuationSource,
        books: Arc<BookCache>,
        life: Lifecycle,
        resolver: crate::service::GiftResolverState,
    ) -> Self {
        Self {
            source,
            books,
            reads: Arc::new(super::read_cache::ReadCache::default()),
            life,
            resolver,
        }
    }
}
impl Actor for MarketValuationActor {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.life.started(ctx);
    }
}
impl Handler<Evaluate<crate::service::Work, Arc<ValuationRead>, ValuationError>>
    for MarketValuationActor
{
    type Result = ResponseFuture<Result<Arc<Mark<Arc<ValuationRead>>>, ValuationError>>;
    fn handle(
        &mut self,
        message: Evaluate<crate::service::Work, Arc<ValuationRead>, ValuationError>,
        _: &mut Self::Context,
    ) -> Self::Result {
        let (mut input, request) = message.into_parts();
        let resolver = self.resolver.get().cloned();
        let source = self.source.clone();
        let books = self.books.clone();
        let reads = self.reads.clone();
        let stop = self.life.stop_token();
        let strategy = self.life.strategy();
        Box::pin(async move {
            let cancelled = request.cancelled.clone();
            let _request = request;
            let work = async move {
                let resolution_failure =
                    super::resolution::external(&mut input.request, resolver).await?;
                let gate = reads.gate(&input.request)?;
                let query = gate.lock_owned().await;
                source
                    .prepare(move |snapshot| -> Result<_, ValuationError> {
                        // Cancellation can drop the awaiting future while its CPU
                        // job is still exiting. Keep publication coalesced until
                        // that job actually releases its admitted read permit.
                        let _query = query;
                        let previous = reads.get(&input.request)?.or(input.previous);
                        let output = ValuationRead::read(
                            &snapshot,
                            &input.request,
                            &books,
                            previous.as_ref(),
                        )
                        .map_err(|error| {
                            if error.code() == ValuationErrorCode::NotFound {
                                resolution_failure.unwrap_or(error)
                            } else {
                                error
                            }
                        })?;
                        reads.insert(input.request, &output)?;
                        let stale = snapshot.sync()?.state != market_gifts::SyncState::Live;
                        Ok(Arc::new(Mark::evaluated(
                            strategy,
                            Prepared::new(output, snapshot.revision()?).with_stale(stale),
                        )))
                    })
                    .await?
            };
            tokio::select! {biased;
                _=stop.cancelled()=>Err(ValuationError::unavailable("Valuation service is stopping")),
                _=cancelled.cancelled()=>Err(ValuationError::unavailable("Valuation request cancelled")),
                result=tokio::time::timeout(Duration::from_secs(30),work)=>result.map_err(|_|ValuationError::timed_out("Valuation report timed out"))?,
            }
        })
    }
}
