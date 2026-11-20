//! One supervised actor owns request admission; preparation runs on the shared read executor.
use super::{GiftFloorMark, GiftFloorValuation, statistics::BookCache};
use crate::{
    Evaluate, GiftValuationRequest, Lifecycle, Mark, Prepared, ValuationError,
    source::{BookScope, ValuationSource},
};
use actix::{Actor, Context, Handler, ResponseFuture};
use std::{sync::Arc, time::Duration};
const READ_TIMEOUT: Duration = Duration::from_secs(30);

pub struct GiftFloorActor {
    life: Lifecycle,
    source: ValuationSource,
    books: Arc<BookCache>,
}
impl GiftFloorActor {
    pub fn new(source: ValuationSource, life: Lifecycle) -> Self {
        Self {
            life,
            source,
            books: Arc::new(BookCache::default()),
        }
    }
}
impl Actor for GiftFloorActor {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.life.started(ctx);
    }
}
impl Handler<Evaluate<GiftValuationRequest, GiftFloorValuation, ValuationError>>
    for GiftFloorActor
{
    type Result = ResponseFuture<Result<Arc<GiftFloorMark>, ValuationError>>;
    fn handle(
        &mut self,
        message: Evaluate<GiftValuationRequest, GiftFloorValuation, ValuationError>,
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
                snapshot.require_currency(&input.currency_id)?;
                let gift = snapshot.gift(&input.gift)?;
                let scope = BookScope {
                    collection: gift.collection_id.clone(),
                    currency: input.currency_id.clone(),
                };
                let stats = books.get(&snapshot, &scope)?;
                let data = stats.report(gift, input.gift, input.currency_id);
                let stale = snapshot.sync()?.state != market_gifts::SyncState::Live;
                snapshot.check_cancelled()?;
                Ok(Arc::new(Mark::evaluated(
                    strategy,
                    Prepared::new(data, snapshot.revision()?).with_stale(stale),
                )))
            });
            tokio::select! { biased;
                _ = stop.cancelled() => Err(ValuationError::unavailable("Valuation actor is stopping")),
                _ = cancelled.cancelled() => Err(ValuationError::unavailable("Valuation request cancelled")),
                result = tokio::time::timeout(READ_TIMEOUT, work) => result.map_err(|_| ValuationError::timed_out("Gift valuation timed out"))??,
            }
        })
    }
}
