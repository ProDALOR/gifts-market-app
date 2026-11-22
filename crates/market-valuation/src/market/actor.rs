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
                let mut resolution_failure = None;
                if let Some(resolver) = resolver
                    && let ValuationTarget::Gift {
                        gift: market_gifts::GiftLookup::Slug(slug),
                    } = &input.request.target
                {
                    // Leave time for a local fallback within the shared request deadline.
                    let resolved =
                        tokio::time::timeout(Duration::from_secs(10), resolver.resolve(slug))
                            .await
                            .unwrap_or(Err(market_gifts::GiftResolutionError::Unavailable));
                    match resolved {
                        Ok(gift) if &gift.slug == slug && gift.number == slug.number() => {
                            input.request.external_gift = Some(gift);
                        }
                        Ok(_) => {
                            return Err(ValuationError::unavailable(
                                "Gift resolver returned a different gift",
                            ));
                        }
                        Err(market_gifts::GiftResolutionError::NotFound) => {
                            return Err(tonic::Status::not_found(
                                "Telegram gift slug was not found",
                            )
                            .into());
                        }
                        Err(error) => {
                            resolution_failure = Some(ValuationError::unavailable(match error {
                                market_gifts::GiftResolutionError::Authorization => {
                                    "Telegram gift lookup session is not authorized".to_owned()
                                }
                                market_gifts::GiftResolutionError::RateLimited(seconds) => format!(
                                    "Telegram gift lookup is rate limited; retry in {seconds}s"
                                ),
                                _ => "Telegram gift lookup is temporarily unavailable".to_owned(),
                            }))
                        }
                    }
                }
                source
                    .prepare(move |snapshot| -> Result<_, ValuationError> {
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
