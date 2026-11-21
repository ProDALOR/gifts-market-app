use super::*;
use crate::{floors::BookCache, source::ValuationSource};
use actix::{Actor, Context, Handler, ResponseFuture};
use std::{sync::Arc, time::Duration};
pub(crate) struct MarketValuationActor {
    source: ValuationSource,
    books: Arc<BookCache>,
    reads: Arc<super::read_cache::ReadCache>,
    life: Lifecycle,
}
impl MarketValuationActor {
    pub fn new(source: ValuationSource, books: Arc<BookCache>, life: Lifecycle) -> Self {
        Self {
            source,
            books,
            reads: Arc::new(super::read_cache::ReadCache::default()),
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
impl Handler<Evaluate<crate::service::Work, Arc<ValuationRead>, ValuationError>>
    for MarketValuationActor
{
    type Result = ResponseFuture<Result<Arc<Mark<Arc<ValuationRead>>>, ValuationError>>;
    fn handle(
        &mut self,
        message: Evaluate<crate::service::Work, Arc<ValuationRead>, ValuationError>,
        _: &mut Self::Context,
    ) -> Self::Result {
        let (input, request) = message.into_parts();
        let source = self.source.clone();
        let books = self.books.clone();
        let reads = self.reads.clone();
        let stop = self.life.stop_token();
        let strategy = self.life.strategy();
        Box::pin(async move {
            let cancelled = request.cancelled.clone();
            let _request = request;
            let work = source.prepare(move |snapshot| -> Result<_, ValuationError> {
                let previous = reads.get(&input.request)?.or(input.previous);
                let output =
                    ValuationRead::read(&snapshot, &input.request, &books, previous.as_ref())?;
                reads.insert(input.request, &output)?;
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
