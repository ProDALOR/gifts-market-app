//! The two typed multiplier actors share one implementation of their common algorithm.
use crate::{Evaluate, Lifecycle, Mark, Prepared};
use actix::{Actor, ActorFutureExt, AsyncContext, AtomicResponse, Context, Handler, WrapFuture};
use market_actors::{
    AskQuote, BookScope, CatalogTraits, OrderbookReader, OrderbookSnapshot, ReadDependencies,
    error::{DataError, DataErrorCode},
};
use market_gifts::{valuation::*, *};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};
use tokio::time::Instant;
const REFRESH: Duration = Duration::from_secs(30);
const IDLE: Duration = Duration::from_secs(300);
const PREPARE_TIMEOUT: Duration = Duration::from_secs(10);
const BOOK_LIMIT: usize = 8;
struct Statistics<I> {
    scope: BookScope,
    known: BTreeSet<I>,
    collection: Option<AskEvidence>,
    traits: BTreeMap<I, AskEvidence>,
    dependencies: ReadDependencies,
    sync: SyncStatus,
}
struct Book<I> {
    prepared: Arc<Prepared<Statistics<I>>>,
    used: Instant,
    created: Instant,
}
fn remember<I>(books: &mut HashMap<BookScope, Book<I>>, prepared: Arc<Prepared<Statistics<I>>>) {
    let scope = prepared.data.scope.clone();
    if books.len() >= BOOK_LIMIT
        && !books.contains_key(&scope)
        && let Some(old) = books
            .iter()
            .min_by_key(|(_, b)| b.used)
            .map(|(s, _)| s.clone())
    {
        books.remove(&old);
    }
    books.insert(
        scope,
        Book {
            prepared,
            used: Instant::now(),
            created: Instant::now(),
        },
    );
}
fn statistics<I: Ord>(
    snapshot: OrderbookSnapshot,
    scope: BookScope,
    known: fn(CatalogTraits) -> BTreeSet<I>,
    selected: fn(AskQuote) -> Option<I>,
) -> Result<Prepared<Statistics<I>>, DataError> {
    snapshot.require_currency(&scope.currency)?;
    let known = known(snapshot.traits(&scope.collection)?);
    let dependencies = snapshot.dependencies([scope.clone()], &scope.currency, false)?;
    let mut data = Statistics {
        scope,
        known,
        collection: None,
        traits: BTreeMap::new(),
        dependencies,
        sync: snapshot.sync()?,
    };
    let now = jiff::Timestamp::now().as_millisecond();
    let mut token = String::new();
    for _ in 0..2560 {
        let page = snapshot.asks(&data.scope, now, token)?;
        for quote in page.data {
            let baseline = data.collection.get_or_insert_with(|| AskEvidence {
                floor: quote.price.clone(),
                samples: 0,
            });
            baseline.samples += 1;
            let price = quote.price.clone();
            if let Some(id) = selected(quote) {
                data.traits
                    .entry(id)
                    .or_insert(AskEvidence {
                        floor: price,
                        samples: 0,
                    })
                    .samples += 1;
            }
        }
        if page.next_page_token.is_empty() {
            return Ok(Prepared::new(data, page.cursor));
        }
        token = page.next_page_token;
    }
    Err(DataError::capacity("multiplier scan limit exceeded"))
}
async fn prepare<I: Ord + Send + 'static>(
    source: OrderbookReader,
    scope: BookScope,
    known: fn(CatalogTraits) -> BTreeSet<I>,
    selected: fn(AskQuote) -> Option<I>,
) -> Result<Arc<Prepared<Statistics<I>>>, DataError> {
    tokio::time::timeout(
        PREPARE_TIMEOUT,
        source.prepare(move |snapshot| statistics(snapshot, scope, known, selected)),
    )
    .await
    .map_err(|_| DataError::timed_out("Multiplier preparation timed out"))?
    .map(Arc::new)
}
fn measure<I: Ord>(
    prepared: &Prepared<Statistics<I>>,
    id: &I,
    strategy: StrategyId,
    selected_trait: ValuationTrait,
) -> Result<(Multiplier, MultiplierEvidence), DataError> {
    let statistics = &prepared.data;
    if !statistics.known.contains(id) {
        return Err(DataError::invalid(
            "selected trait does not belong to this collection",
        ));
    }
    let selected = statistics.traits.get(id);
    let collection = statistics.collection.as_ref();
    let trait_samples = selected.map_or(0, |e| e.samples);
    let collection_samples = collection.map_or(0, |e| e.samples);
    let details = ValuationErrorDetails {
        strategy,
        currency_id: Some(statistics.scope.currency.clone()),
        revision: Some(prepared.revision.clone()),
        required_samples: Some(MINIMUM_MULTIPLIER_SAMPLES),
        available_samples: Some(trait_samples.min(collection_samples)),
        context: Some(ValuationErrorContext::Multiplier {
            collection_id: statistics.scope.collection.clone(),
            selected_trait,
            collection_floor: collection.map(|e| e.floor.clone()),
            trait_floor: selected.map(|e| e.floor.clone()),
            collection_samples,
            trait_samples,
        }),
    };
    if collection_samples < MINIMUM_MULTIPLIER_SAMPLES || trait_samples < MINIMUM_MULTIPLIER_SAMPLES
    {
        return Err(DataError::valuation(
            DataErrorCode::NotEnoughData,
            "At least three eligible asks are required for both the collection and selected trait",
            details,
        ));
    }
    let (Some(baseline), Some(trait_floor)) = (collection, selected) else {
        return Err(DataError::valuation(
            DataErrorCode::InvalidBaseline,
            "Multiplier floor evidence is missing",
            details,
        ));
    };
    let factor = Multiplier::from_floors(&trait_floor.floor, &baseline.floor).map_err(|e| {
        DataError::valuation(DataErrorCode::InvalidBaseline, e.to_string(), details)
    })?;
    Ok((
        factor,
        MultiplierEvidence {
            collection: baseline.clone(),
            selected_trait: trait_floor.clone(),
            minimum_samples: MINIMUM_MULTIPLIER_SAMPLES,
        },
    ))
}

// Only the selected trait and public request/result types differ. Keep scheduling and
// freshness policy in one place while exposing ordinary, independently started actors.
macro_rules! multiplier_actor {
    ($actor:ident,$request:ident,$value:ident,$trait_id:ty,$field:ident,$catalog:ident,$strategy:ident,$variant:ident) => {

        pub struct $actor {
            source: OrderbookReader,
            life: Lifecycle,
            books: HashMap<BookScope, Book<$trait_id>>,
            last: Option<($request, Arc<Mark<$value>>)>,
            active: Option<BookScope>,
            last_requested: Instant,
            refreshing: bool,
        }
        impl $actor {
            pub fn new(source: OrderbookReader, life: Lifecycle) -> Self {
                Self {
                    source,
                    life,
                    books: HashMap::new(),
                    last: None,
                    active: None,
                    last_requested: Instant::now(),
                    refreshing: false,
                }
            }
            fn refresh(&mut self, ctx: &mut Context<Self>) {
                if self.last_requested.elapsed() >= IDLE {
                    self.books.clear();
                    self.last = None;
                    self.active = None;
                    return;
                }
                let Some(scope) = self.active.clone() else {
                    return;
                };
                if self.refreshing
                    || self.books.get(&scope).is_some_and(|b| {
                        b.created.elapsed() < REFRESH
                            && self.source.is_current(&b.prepared.data.dependencies)
                    })
                {
                    return;
                }
                self.refreshing = true;
                let source = self.source.clone();
                let stop = self.life.stop_token();
                let pending = scope.clone();
                let work = async move {
                    tokio::select! {biased;
                        _=stop.cancelled()=>Err(DataError::unavailable("Valuation actor is stopping")),
                        result=prepare(source,pending,|traits|traits.$catalog,|quote|quote.$field)=>result,
                    }
                };
                // A timer refresh must not suspend cached requests in this actor.
                ctx.spawn(work.into_actor(self).map(move |result, actor, _| {
                    actor.refreshing = false;
                    if actor.last_requested.elapsed() >= IDLE {
                        return;
                    }
                    if let Ok(prepared) = result
                        && actor.source.is_current(&prepared.data.dependencies)
                        && actor
                            .books
                            .get(&scope)
                            .is_none_or(|book| book.prepared.prepared_at <= prepared.prepared_at)
                    {
                        actor.last = None;
                        remember(&mut actor.books, prepared);
                    }
                }));
            }
        }
        impl Actor for $actor {
            type Context = Context<Self>;
            fn started(&mut self, ctx: &mut Self::Context) {
                self.life.started(ctx);
                ctx.run_interval(REFRESH, |actor, ctx| actor.refresh(ctx));
            }
        }
        impl Handler<Evaluate<$request, $value, DataError>> for $actor {
            type Result = AtomicResponse<Self, Result<Arc<Mark<$value>>, DataError>>;
            fn handle(
                &mut self,
                message: Evaluate<$request, $value, DataError>,
                _: &mut Self::Context,
            ) -> Self::Result {
                let (input, request) = message.into_parts();
                let scope = BookScope {
                    collection: input.collection_id.clone(),
                    currency: input.currency_id.clone(),
                };
                self.active = Some(scope.clone());
                self.last_requested = Instant::now();
                let source = self.source.clone();
                let cached = self
                    .books
                    .get_mut(&scope)
                    .filter(|book| {
                        book.created.elapsed() < REFRESH
                            && source.is_current(&book.prepared.data.dependencies)
                    })
                    .map(|book| {
                        book.used = Instant::now();
                        book.prepared.clone()
                    });
                let last = cached.as_ref().and_then(|prepared| {
                    self.last
                        .as_ref()
                        .filter(|(key, mark)| key == &input && mark.evaluated_at == prepared.prepared_at)
                        .map(|(_, mark)| mark.clone())
                });
                let stop = self.life.stop_token();
                let pending = scope.clone();
                let work = async move {
                    let _request = request;
                    if let Some(prepared) = cached {
                        return Ok((prepared, false));
                    }
                    tokio::select! {biased;
                        _=stop.cancelled()=>Err(DataError::unavailable("Valuation actor is stopping")),
                        _=_request.cancelled.cancelled()=>Err(DataError::unavailable("Valuation request cancelled")),
                        result=prepare(source,pending,|traits|traits.$catalog,|quote|quote.$field)=>result.map(|p|(p,true)),
                    }
                };
                AtomicResponse::new(Box::pin(work.into_actor(self).map(
                    move |result, actor, _| {
                        let (prepared, fresh) = match result {
                            Ok(value) => value,
                            Err(error) => {
                                actor.books.remove(&scope);
                                actor.last = None;
                                return Err(error);
                            }
                        };
                        if fresh && actor.source.is_current(&prepared.data.dependencies) {
                            remember(&mut actor.books, prepared.clone());
                        }
                        if let Some(mark) = last {
                            return Ok(mark);
                        }
                        let (factor, evidence) = measure(
                            &prepared,
                            &input.$field,
                            StrategyId::$strategy(),
                            ValuationTrait::$variant(input.$field.clone()),
                        )?;
                        let data = $value {
                            collection_id: input.collection_id.clone(),
                            $field: input.$field.clone(),
                            currency_id: input.currency_id.clone(),
                            factor,
                            evidence,
                            sync: prepared.data.sync.clone(),
                        };
                        let mark = Arc::new(Mark::prepared(actor.life.strategy(), data, &prepared));
                        actor.last = Some((input, mark.clone()));
                        Ok(mark)
                    },
                )))
            }
        }

    }
}
multiplier_actor!(
    ModelMultiplierActor,
    ModelMultiplierRequest,
    ModelMultiplier,
    ModelId,
    model_id,
    models,
    model_multiplier,
    Model
);
multiplier_actor!(
    BackdropMultiplierActor,
    BackdropMultiplierRequest,
    BackdropMultiplier,
    BackdropId,
    backdrop_id,
    backdrops,
    backdrop_multiplier,
    Backdrop
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{test_lifecycle, test_message};
    use actix::Addr;
    #[derive(actix::Message)]
    #[rtype(result = "()")]
    struct Refresh;
    impl Handler<Refresh> for ModelMultiplierActor {
        type Result = ();
        fn handle(&mut self, _: Refresh, ctx: &mut Context<Self>) {
            self.refresh(ctx);
        }
    }
    fn input(currency: &str) -> ModelMultiplierRequest {
        ModelMultiplierRequest {
            collection_id: "gift".into(),
            model_id: "a".into(),
            currency_id: currency.to_owned().try_into().unwrap(),
        }
    }
    fn scope(currency: &str) -> BookScope {
        BookScope {
            collection: "gift".into(),
            currency: currency.to_owned().try_into().unwrap(),
        }
    }
    async fn data() -> (tempfile::TempDir, market_actors::MarketDataService) {
        use crate::integration_tests::{entity, order};
        use market_hub_protocol::wire as w;
        let (dir, mut writer) = crate::integration_tests::model_fixture();
        let mut changes = vec![entity(w::entity::Entity::Currency(w::Currency {
            id: "XTR".into(),
            name: "Stars".into(),
            decimals: 0,
        }))];
        for i in 0..3 {
            let mut row = order("20");
            let Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::Order(value)),
            })) = &mut row.change
            else {
                unreachable!()
            };
            value.id = format!("stars.{i}");
            value.book.as_mut().unwrap().currency_id = "XTR".into();
            value.attributes = [(
                "tg.model".into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId("a".into())),
                    })),
                },
            )]
            .into();
            changes.push(row);
        }
        writer
            .apply(w::Batch {
                cursor: "currencies".into(),
                changes,
            })
            .unwrap();
        (
            dir,
            market_actors::MarketDataService::start(writer.reader(), 4, 1)
                .await
                .unwrap(),
        )
    }
    async fn book(
        data: &market_actors::MarketDataService,
        currency: &str,
    ) -> Arc<Prepared<Statistics<ModelId>>> {
        prepare(
            data.orderbooks.clone(),
            scope(currency),
            |traits| traits.models,
            |quote| quote.model_id,
        )
        .await
        .unwrap()
    }
    async fn ask(addr: &Addr<ModelMultiplierActor>, currency: &str) -> Arc<Mark<ModelMultiplier>> {
        addr.send(test_message(input(currency)))
            .await
            .unwrap()
            .unwrap()
    }
    #[actix::test]
    async fn background_refresh_does_not_block_cached_requests_or_shutdown() {
        let (_dir, data) = data().await;
        let gram = book(&data, "GRAM").await;
        let stars = book(&data, "XTR").await;
        let held = crate::integration_tests::hold(&data.orderbooks).await;
        let (life, stop, closed) = test_lifecycle("model_multiplier");
        let mut actor = ModelMultiplierActor::new(data.orderbooks.clone(), life);
        remember(&mut actor.books, gram);
        remember(&mut actor.books, stars);
        actor.books.get_mut(&scope("GRAM")).unwrap().created -= REFRESH;
        actor.active = Some(scope("GRAM"));
        let addr = actor.start();
        addr.send(Refresh).await.unwrap();
        let mark = tokio::time::timeout(Duration::from_secs(1), ask(&addr, "XTR"))
            .await
            .unwrap();
        assert_eq!(mark.factor.as_str(), "1");
        assert_eq!(mark.currency_id.as_str(), "XTR");
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(1), closed.cancelled())
            .await
            .unwrap();
        drop(held);
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
    #[actix::test]
    async fn cached_requests_do_not_extend_multiplier_preparation_freshness() {
        let (_dir, data) = data().await;
        let prepared = book(&data, "GRAM").await;
        let (life, stop, closed) = test_lifecycle("model_multiplier");
        let mut actor = ModelMultiplierActor::new(data.orderbooks.clone(), life);
        remember(&mut actor.books, prepared);
        actor.books.get_mut(&scope("GRAM")).unwrap().created -= Duration::from_millis(29_500);
        let addr = actor.start();
        let first = ask(&addr, "GRAM").await;
        let held = crate::integration_tests::hold(&data.orderbooks).await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        let mut pending = Box::pin(ask(&addr, "GRAM"));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut pending)
                .await
                .is_err()
        );
        drop(held);
        let fresh = pending.await;
        assert!(fresh.evaluated_at > first.evaluated_at);
        stop.cancel();
        closed.cancelled().await;
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
}
