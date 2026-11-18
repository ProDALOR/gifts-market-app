//! Shared multiplier calculation and cache policy; native actors supply typed messages.
use crate::{
    Evaluate, Lifecycle, Mark, Prepared, RequestContext, ValuationError, ValuationErrorCode,
};
use actix::{Actor, ActorFutureExt, AsyncContext, AtomicResponse, Context, Handler, WrapFuture};
use market_actors::{
    AskQuote, BookScope, CatalogTrait, MarketReader, MarketSnapshot, ReadDependencies,
    error::DataError,
};
use market_gifts::{valuation::*, *};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
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
    completed_at: Instant,
}
struct Book<I> {
    prepared: Arc<Prepared<Statistics<I>>>,
    used: Instant,
    created: Instant,
}
struct TraitRead<I> {
    kind: CatalogTrait,
    known: fn(&MarketSnapshot, &CollectionId) -> Result<BTreeSet<I>, DataError>,
    selected: fn(AskQuote) -> Option<I>,
}
impl<I> Copy for TraitRead<I> {}
impl<I> Clone for TraitRead<I> {
    fn clone(&self) -> Self {
        *self
    }
}
struct Work<I> {
    source: MarketReader,
    scope: BookScope,
    candidate: Option<(Arc<Prepared<Statistics<I>>>, Instant)>,
    read: TraitRead<I>,
    started: Instant,
}
struct Outcome<I> {
    prepared: Arc<Prepared<Statistics<I>>>,
    fresh: bool,
    current: bool,
}
struct MultiplierBooks<I> {
    source: MarketReader,
    read: TraitRead<I>,
    books: HashMap<BookScope, Book<I>>,
    active: Option<BookScope>,
    last_requested: Instant,
    refreshing: bool,
}
impl<I> MultiplierBooks<I> {
    fn new(source: MarketReader, read: TraitRead<I>) -> Self {
        Self {
            source,
            read,
            books: HashMap::new(),
            active: None,
            last_requested: Instant::now(),
            refreshing: false,
        }
    }
    fn work(&mut self, scope: BookScope) -> Work<I> {
        let candidate = self
            .books
            .get_mut(&scope)
            .filter(|book| book.created.elapsed() < REFRESH)
            .map(|book| {
                book.used = Instant::now();
                (book.prepared.clone(), book.created)
            });
        Work {
            source: self.source.clone(),
            scope,
            candidate,
            read: self.read,
            started: Instant::now(),
        }
    }
    fn request(&mut self, scope: BookScope) -> Work<I> {
        self.active = Some(scope.clone());
        self.last_requested = Instant::now();
        self.work(scope)
    }
    fn expire(&mut self) -> bool {
        if self.last_requested.elapsed() < IDLE {
            return false;
        }
        self.books.clear();
        self.active = None;
        true
    }
    fn refresh(&mut self) -> Option<Work<I>> {
        if self.refreshing {
            return None;
        }
        let scope = self.active.clone()?;
        self.refreshing = true;
        Some(self.work(scope))
    }
    fn discard(&mut self, scope: &BookScope, started: Instant) {
        if self
            .books
            .get(scope)
            .is_some_and(|book| book.created <= started)
        {
            self.books.remove(scope);
        }
    }
    fn accept(&mut self, outcome: &Outcome<I>) -> bool {
        if !outcome.fresh
            || !outcome.current
            || outcome.prepared.data.completed_at.elapsed() >= REFRESH
        {
            return false;
        }
        let scope = outcome.prepared.data.scope.clone();
        if self
            .books
            .get(&scope)
            .is_some_and(|book| book.prepared.prepared_at > outcome.prepared.prepared_at)
        {
            return false;
        }
        if self.books.len() >= BOOK_LIMIT
            && !self.books.contains_key(&scope)
            && let Some(old) = self
                .books
                .iter()
                .min_by_key(|(_, b)| b.used)
                .map(|(scope, _)| scope.clone())
        {
            self.books.remove(&old);
        }
        self.books.insert(
            scope,
            Book {
                prepared: outcome.prepared.clone(),
                created: outcome.prepared.data.completed_at,
                used: Instant::now(),
            },
        );
        true
    }
}
fn statistics<I: Ord>(
    snapshot: MarketSnapshot,
    scope: BookScope,
    read: TraitRead<I>,
) -> Result<Prepared<Statistics<I>>, ValuationError> {
    snapshot.require_currency(&scope.currency)?;
    let known = (read.known)(&snapshot, &scope.collection)?;
    let dependencies = snapshot.dependencies([scope.clone()], &scope.currency, read.kind, &[])?;
    let mut data = Statistics {
        scope,
        known,
        collection: None,
        traits: BTreeMap::new(),
        dependencies,
        sync: snapshot.sync()?,
        completed_at: Instant::now(),
    };
    let now = jiff::Timestamp::now().as_millisecond();
    let mut token = String::new();
    for _ in 0..2560 {
        let page = snapshot.asks(&data.scope, now, token)?;
        for quote in page.data {
            data.collection
                .get_or_insert_with(|| AskEvidence {
                    floor: quote.price.clone(),
                    samples: 0,
                })
                .samples += 1;
            let price = quote.price.clone();
            if let Some(id) = (read.selected)(quote) {
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
            data.completed_at = Instant::now();
            return Ok(Prepared::new(data, page.cursor));
        }
        token = page.next_page_token;
    }
    Err(ValuationError::capacity("multiplier scan limit exceeded"))
}
async fn resolve<I: Ord + Send + 'static>(work: Work<I>) -> Result<Outcome<I>, ValuationError> {
    if let Some((prepared, created)) = work.candidate
        && work.source.is_current(&prepared.data.dependencies).await
        && created.elapsed() < REFRESH
    {
        return Ok(Outcome {
            prepared,
            fresh: false,
            current: true,
        });
    }
    let read = work.read;
    let scope = work.scope;
    let prepared = Arc::new(
        tokio::time::timeout(
            PREPARE_TIMEOUT,
            work.source
                .prepare(move |snapshot| statistics(snapshot, scope, read)),
        )
        .await
        .map_err(|_| ValuationError::timed_out("Multiplier preparation timed out"))???,
    );
    let current = work.source.is_current(&prepared.data.dependencies).await;
    Ok(Outcome {
        prepared,
        fresh: true,
        current,
    })
}
async fn request<I: Ord + Send + 'static>(
    work: Work<I>,
    context: RequestContext,
    stop: CancellationToken,
) -> Result<Outcome<I>, ValuationError> {
    let cancelled = context.cancelled.clone();
    let _context = context;
    tokio::select! {biased;
        _=stop.cancelled()=>Err(ValuationError::unavailable("Valuation actor is stopping")),
        _=cancelled.cancelled()=>Err(ValuationError::unavailable("Valuation request cancelled")),
        result=resolve(work)=>result,
    }
}
fn measure<I: Ord>(
    prepared: &Prepared<Statistics<I>>,
    id: &I,
    strategy: StrategyId,
    selected_trait: ValuationTrait,
) -> Result<(Multiplier, MultiplierEvidence), ValuationError> {
    let statistics = &prepared.data;
    if !statistics.known.contains(id) {
        return Err(ValuationError::invalid(
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
        return Err(ValuationError::with_details(
            ValuationErrorCode::NotEnoughData,
            "At least three eligible asks are required for both the collection and selected trait",
            details,
        ));
    }
    let (Some(baseline), Some(trait_floor)) = (collection, selected) else {
        return Err(ValuationError::with_details(
            ValuationErrorCode::InvalidBaseline,
            "Multiplier floor evidence is missing",
            details,
        ));
    };
    let factor = Multiplier::from_floors(&trait_floor.floor, &baseline.floor).map_err(|e| {
        ValuationError::with_details(ValuationErrorCode::InvalidBaseline, e.to_string(), details)
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

// Only native actor/message boilerplate is generated. Policy lives in MultiplierBooks.
macro_rules! multiplier_actor {
    ($actor:ident,$request:ident,$value:ident,$id:ty,$field:ident,$catalog:ident,$kind:ident,$strategy:ident,$variant:ident)=>{

        pub struct $actor {
            life: Lifecycle,
            state: MultiplierBooks<$id>,
            last: Option<($request, Arc<Mark<$value>>)>,
        }
        impl $actor {
            pub fn new(source: MarketReader, life: Lifecycle) -> Self {
                Self {
                    life,
                    state: MultiplierBooks::new(
                        source,
                        TraitRead {
                            kind: CatalogTrait::$kind,
                            known: MarketSnapshot::$catalog,
                            selected: |quote| quote.$field,
                        },
                    ),
                    last: None,
                }
            }
            fn refresh(&mut self, ctx: &mut Context<Self>) {
                if self.state.expire() {
                    self.last = None;
                    return;
                }
                let Some(work) = self.state.refresh() else {
                    return;
                };
                let scope = work.scope.clone();
                let started = work.started;
                let stop = self.life.stop_token();
                let task = async move {
                    tokio::select! {biased;_=stop.cancelled()=>Err(ValuationError::unavailable("Valuation actor is stopping")),result=resolve(work)=>result}
                };
                ctx.spawn(task.into_actor(self).map(move |result, actor, _| {
                    actor.state.refreshing = false;
                    if actor.state.expire() {
                        actor.last = None;
                        return;
                    }
                    match result {
                        Ok(outcome) => {
                            if actor.state.accept(&outcome) {
                                actor.last = None;
                            }
                        }
                        Err(_) => actor.state.discard(&scope, started),
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
        impl Handler<Evaluate<$request, $value, ValuationError>> for $actor {
            type Result = AtomicResponse<Self, Result<Arc<Mark<$value>>, ValuationError>>;
            fn handle(
                &mut self,
                message: Evaluate<$request, $value, ValuationError>,
                _: &mut Self::Context,
            ) -> Self::Result {
                let (input, context) = message.into_parts();
                let scope = BookScope {
                    collection: input.collection_id.clone(),
                    currency: input.currency_id.clone(),
                };
                let work = self.state.request(scope.clone());
                let started = work.started;
                AtomicResponse::new(Box::pin(
                    request(work, context, self.life.stop_token())
                        .into_actor(self)
                        .map(move |result, actor, _| {
                            let outcome = result.map_err(|error| {
                                actor.state.discard(&scope, started);
                                error
                            })?;
                            actor.state.accept(&outcome);
                            let prepared = &outcome.prepared;
                            if let Some((key, mark)) = &actor.last
                                && key == &input
                                && mark.evaluated_at == prepared.prepared_at
                            {
                                return Ok(mark.clone());
                            }
                            let (factor, evidence) = measure(
                                prepared,
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
                            let mark = Arc::new(Mark::prepared(actor.life.strategy(), data, prepared));
                            actor.last = Some((input, mark.clone()));
                            Ok(mark)
                        }),
                ))
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
    model_ids,
    Model,
    model_multiplier,
    Model
);
multiplier_actor!(
    BackdropMultiplierActor,
    BackdropMultiplierRequest,
    BackdropMultiplier,
    BackdropId,
    backdrop_id,
    backdrop_ids,
    Backdrop,
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
        let work = Work {
            source: data.reader.clone(),
            scope: scope(currency),
            candidate: None,
            read: TraitRead {
                kind: CatalogTrait::Model,
                known: MarketSnapshot::model_ids,
                selected: |quote| quote.model_id,
            },
            started: Instant::now(),
        };
        resolve(work).await.unwrap().prepared
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
        let held = crate::integration_tests::hold(&data.reader).await;
        let (life, stop, closed) = test_lifecycle("model_multiplier");
        let mut actor = ModelMultiplierActor::new(data.reader.clone(), life);
        actor.state.accept(&Outcome {
            prepared: gram,
            fresh: true,
            current: true,
        });
        actor.state.accept(&Outcome {
            prepared: stars,
            fresh: true,
            current: true,
        });
        actor.state.books.get_mut(&scope("GRAM")).unwrap().created -= REFRESH;
        actor.state.active = Some(scope("GRAM"));
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
        let mut actor = ModelMultiplierActor::new(data.reader.clone(), life);
        actor.state.accept(&Outcome {
            prepared,
            fresh: true,
            current: true,
        });
        actor.state.books.get_mut(&scope("GRAM")).unwrap().created -= Duration::from_millis(29_500);
        let addr = actor.start();
        let first = ask(&addr, "GRAM").await;
        let held = crate::integration_tests::hold(&data.reader).await;
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
