use crate::{
    cache::{Cache, Snapshot},
    queue::Queue,
    service::{Cancel, Submit},
    *,
};
use actix::prelude::*;
use market_gifts::{
    CurrencyId,
    valuation::{ModelLookup, ModelPosition, ValuationRequest},
};
use market_invocations::{
    Control, Error as InvocationError, Handle as InvocationHandle, Options, RuntimeError,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct QuoteKey {
    user: UserId,
    currency: CurrencyId,
    snapshot: u64,
    strategy: market_gifts::valuation::StrategyId,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Work {
    Inventory(UserId),
    Value(QuoteKey),
}
struct Pending {
    submit: Submit,
    work: Work,
}
struct InventoryJob {
    waiters: Vec<u64>,
    data: crate::inventory::InventoryBuilder,
}
impl InventoryJob {
    fn new(id: u64) -> Self {
        Self {
            waiters: vec![id],
            data: Default::default(),
        }
    }
}
struct ValuationJob {
    waiters: Vec<u64>,
    snapshot: Snapshot,
}
struct PortfolioActor<P, V> {
    provider: Arc<P>,
    valuation: Arc<V>,
    config: PortfolioConfig,
    pending: HashMap<u64, Pending>,
    inventories: HashMap<UserId, InventoryJob>,
    values: HashMap<QuoteKey, ValuationJob>,
    queue: Queue<Work>,
    cache: Cache,
    next_snapshot: u64,
    fetch: HashMap<UserId, (SpawnHandle, Control)>,
    invocations: InvocationHandle,
    value: Option<(QuoteKey, SpawnHandle)>,
    stop: CancellationToken,
    closed: CancellationToken,
}
pub(crate) fn start<P: InventoryProvider, V: ValuationProvider>(
    provider: P,
    valuation: V,
    config: PortfolioConfig,
    invocations: InvocationHandle,
    stop: CancellationToken,
    closed: CancellationToken,
) -> PortfolioHandle {
    let actor = PortfolioActor {
        provider: Arc::new(provider),
        valuation: Arc::new(valuation),
        cache: Cache::new(config.cached_users, config.inventory_ttl),
        config: config.clone(),
        pending: HashMap::new(),
        inventories: HashMap::new(),
        values: HashMap::new(),
        queue: Queue::default(),
        next_snapshot: 0,
        fetch: HashMap::new(),
        invocations,
        value: None,
        stop: stop.clone(),
        closed: closed.clone(),
    };
    let mut context = Context::new();
    context.set_mailbox_capacity(config.max_requests);
    let actor = context.run(actor);
    PortfolioHandle::new(
        actor.clone().recipient(),
        actor.recipient(),
        &config,
        stop,
        closed,
    )
}
impl<P: InventoryProvider, V: ValuationProvider> Actor for PortfolioActor<P, V> {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        let invocations = self.invocations.clone();
        ctx.spawn(
            async move { invocations.closed().await }
                .into_actor(self)
                .map(|_, _, ctx| ctx.stop()),
        );
        ctx.spawn(
            self.stop
                .clone()
                .cancelled_owned()
                .into_actor(self)
                .map(|_, _, ctx| ctx.stop()),
        );
    }
}
impl<P, V> Drop for PortfolioActor<P, V> {
    fn drop(&mut self) {
        self.closed.cancel();
    }
}
impl<P: InventoryProvider, V: ValuationProvider> Handler<Submit> for PortfolioActor<P, V> {
    type Result = ();
    fn handle(&mut self, submit: Submit, ctx: &mut Context<Self>) {
        if self.stop.is_cancelled() || submit.reply.is_closed() {
            let _ = submit.reply.send(Err(PortfolioError::Stopped));
            return;
        }
        let id = submit.id;
        let user = submit.request.user;
        let work = Work::Inventory(user);
        self.pending.insert(
            id,
            Pending {
                submit,
                work: work.clone(),
            },
        );
        if let Some(snapshot) = self.cache.get(user) {
            self.enqueue_value(id, snapshot);
        } else if let Some(job) = self.inventories.get_mut(&user) {
            job.waiters.push(id);
            self.requeue(work);
        } else {
            self.inventories.insert(user, InventoryJob::new(id));
            self.requeue(work);
        }
        self.pump(ctx);
    }
}
impl<P: InventoryProvider, V: ValuationProvider> Handler<Cancel> for PortfolioActor<P, V> {
    type Result = ();
    fn handle(&mut self, Cancel(id): Cancel, ctx: &mut Context<Self>) {
        self.cancel_request(id, ctx);
        self.pump(ctx);
    }
}
impl<P: InventoryProvider, V: ValuationProvider> PortfolioActor<P, V> {
    fn cancel_request(&mut self, id: u64, ctx: &mut Context<Self>) {
        let Some(pending) = self.pending.remove(&id) else {
            return;
        };
        match pending.work {
            Work::Inventory(user) => {
                if let Some(job) = self.inventories.get_mut(&user) {
                    job.waiters.retain(|waiter| *waiter != id);
                    if job.waiters.is_empty() {
                        self.inventories.remove(&user);
                        self.queue.remove(&Work::Inventory(user));
                        if let Some((handle, _)) = self.fetch.remove(&user) {
                            ctx.cancel_future(handle);
                        }
                    } else {
                        self.requeue(Work::Inventory(user));
                    }
                }
            }
            Work::Value(key) => {
                if let Some(job) = self.values.get_mut(&key) {
                    job.waiters.retain(|waiter| *waiter != id);
                    // An active actor read retains its capacity until completion, even without waiters.
                    if job.waiters.is_empty()
                        && !self
                            .value
                            .as_ref()
                            .is_some_and(|(active, _)| *active == key)
                    {
                        self.values.remove(&key);
                        self.queue.remove(&Work::Value(key));
                    } else {
                        self.requeue(Work::Value(key));
                    }
                }
            }
        }
    }
    fn requeue(&mut self, work: Work) {
        if self
            .value
            .as_ref()
            .is_some_and(|(key, _)| work == Work::Value(key.clone()))
        {
            return;
        }
        let waiters = match &work {
            Work::Inventory(user) => self.inventories.get(user).map(|j| &j.waiters),
            Work::Value(key) => self.values.get(key).map(|j| &j.waiters),
        };
        let priority = waiters
            .into_iter()
            .flatten()
            .filter_map(|id| self.pending.get(id))
            .map(|p| p.submit.request.priority)
            .min();
        if let Some(priority) = priority {
            if let Work::Inventory(user) = &work
                && let Some((_, control)) = self.fetch.get(user)
            {
                control.set_priority(priority);
            } else {
                self.queue.insert(work, priority);
            }
        }
    }
    fn enqueue_value(&mut self, id: u64, snapshot: Snapshot) {
        let Some(pending) = self.pending.get_mut(&id) else {
            return;
        };
        let key = QuoteKey {
            user: pending.submit.request.user,
            currency: pending.submit.request.currency_id.clone(),
            snapshot: snapshot.id,
            strategy: pending.submit.request.strategy.clone(),
        };
        pending.work = Work::Value(key.clone());
        self.values
            .entry(key.clone())
            .or_insert_with(|| ValuationJob {
                waiters: vec![],
                snapshot,
            })
            .waiters
            .push(id);
        self.requeue(Work::Value(key));
    }
    fn finish(&mut self, id: u64, result: Result<Portfolio, PortfolioError>) {
        if let Some(pending) = self.pending.remove(&id) {
            let _ = pending.submit.reply.send(result);
        }
    }
    fn pump(&mut self, ctx: &mut Context<Self>) {
        let abandoned: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.submit.reply.is_closed())
            .map(|(id, _)| *id)
            .collect();
        for id in abandoned {
            self.cancel_request(id, ctx);
        }
        if self.stop.is_cancelled() {
            ctx.stop();
            return;
        }
        loop {
            let value_ready = self.value.is_none();
            let work = self.queue.pop_ready(|work| match work {
                Work::Inventory(user) => !self.fetch.contains_key(user),
                Work::Value(_) => value_ready,
            });
            match work {
                Some(Work::Inventory(user)) => self.fetch_page(user, ctx),
                Some(Work::Value(key)) => self.value_snapshot(key, ctx),
                None => break,
            }
        }
    }
    fn fetch_page(&mut self, user: UserId, ctx: &mut Context<Self>) {
        let job = &self.inventories[&user];
        let request = InventoryPageRequest {
            user,
            cursor: job.data.cursor.clone(),
            limit: self
                .config
                .page_size
                .min(self.config.max_gifts - job.data.scanned),
        };
        let limit = request.limit;
        let priority = job
            .waiters
            .iter()
            .filter_map(|id| self.pending.get(id))
            .map(|p| p.submit.request.priority)
            .min()
            .unwrap_or_default();
        let ticket = self.invocations.submit(
            self.provider.page(request),
            Options {
                priority,
                timeout: self.config.request_timeout,
                cancellation: self.stop.clone(),
            },
        );
        let ticket = match ticket {
            Ok(ticket) => ticket,
            Err(error) => {
                let error = match error {
                    RuntimeError::Busy => PortfolioError::Busy,
                    _ => PortfolioError::Stopped,
                };
                if let Some(job) = self.inventories.remove(&user) {
                    for id in job.waiters {
                        self.finish(id, Err(error.clone()));
                    }
                }
                return;
            }
        };
        let control = ticket.control();
        let handle = ctx.spawn(
            async move {
                ticket.await.map_err(|error| match error {
                    InvocationError::Operation(error) => PortfolioError::Inventory(error),
                    InvocationError::Runtime(error) => match error {
                        RuntimeError::Deadline { .. } => PortfolioError::TimedOut,
                        RuntimeError::Busy => PortfolioError::Busy,
                        RuntimeError::Stopped | RuntimeError::Cancelled => PortfolioError::Stopped,
                        _ => PortfolioError::Inventory(InventoryError::Unavailable),
                    },
                })
            }
            .into_actor(self)
            .map(move |result, actor, ctx| {
                actor.fetch.remove(&user);
                match result {
                    Ok(page) => actor.page_finished(user, limit, Ok(page)),
                    Err(PortfolioError::Inventory(error)) => {
                        actor.page_finished(user, limit, Err(error))
                    }
                    Err(error) => {
                        if let Some(job) = actor.inventories.remove(&user) {
                            for id in job.waiters {
                                actor.finish(id, Err(error.clone()));
                            }
                        }
                    }
                }
                actor.pump(ctx);
            }),
        );
        self.fetch.insert(user, (handle, control));
    }
    fn page_finished(
        &mut self,
        user: UserId,
        limit: usize,
        result: Result<InventoryPage, InventoryError>,
    ) {
        let completion = match result {
            Err(error) => Err(error),
            Ok(page) => {
                self.inventories
                    .get_mut(&user)
                    .unwrap()
                    .data
                    .merge(page, limit, &self.config)
            }
        };
        match completion {
            Ok(None) => self.requeue(Work::Inventory(user)),
            Ok(Some(completion)) => self.inventory_finished(user, Ok(completion)),
            Err(error) => self.inventory_finished(user, Err(error)),
        }
    }
    fn inventory_finished(
        &mut self,
        user: UserId,
        completion: Result<InventoryCompletion, InventoryError>,
    ) {
        let Some(mut job) = self.inventories.remove(&user) else {
            return;
        };
        self.queue.remove(&Work::Inventory(user));
        let completion = match completion {
            Ok(completion) => completion,
            Err(error)
                if job.data.pages > 0
                    && error != InventoryError::Authorization
                    && error != InventoryError::Forbidden =>
            {
                InventoryCompletion::Partial(PartialReason::ReadFailed(error))
            }
            Err(error) => {
                for id in job.waiters {
                    self.finish(id, Err(PortfolioError::Inventory(error.clone())));
                }
                return;
            }
        };
        let waiters = std::mem::take(&mut job.waiters);
        let Some(next) = self.next_snapshot.checked_add(1) else {
            for id in waiters {
                self.finish(id, Err(PortfolioError::Stopped));
            }
            return;
        };
        self.next_snapshot = next;
        let complete = completion == InventoryCompletion::Complete;
        let snapshot = Snapshot {
            id: next,
            inventory: Arc::new(job.data.snapshot(completion)),
        };
        if complete {
            self.cache.insert(user, snapshot.clone());
        }
        for id in waiters {
            self.enqueue_value(id, snapshot.clone());
        }
    }
    fn value_snapshot(&mut self, key: QuoteKey, ctx: &mut Context<Self>) {
        let job = &self.values[&key];
        let snapshot = job.snapshot.clone();
        let backend = self.valuation.clone();
        let timeout = self.config.valuation_timeout;
        let mut positions: BTreeMap<ModelLookup, u32> = BTreeMap::new();
        for gift in &snapshot.inventory.gifts {
            *positions
                .entry(ModelLookup::Telegram {
                    gift_id: gift.collection_id,
                    document_id: gift.model_document_id,
                })
                .or_default() += 1;
        }
        let request = ValuationRequest {
            strategy: key.strategy.clone(),
            positions: positions
                .into_iter()
                .map(|(model, quantity)| ModelPosition { model, quantity })
                .collect(),
            currency_id: key.currency.clone(),
        };
        let strategy = request.strategy.clone();
        let currency = request.currency_id.clone();
        let completed_key = key.clone();
        let handle = ctx.spawn(
            async move {
                tokio::time::timeout(timeout, backend.mark(request))
                    .await
                    .unwrap_or_else(|_| {
                        Err(market_actors::error::DataError::timed_out(
                            "portfolio valuation timed out",
                        )
                        .with_valuation(strategy, currency))
                    })
            }
            .into_actor(self)
            .map(move |result, actor, ctx| {
                actor.value = None;
                let job = actor.values.remove(&completed_key).unwrap();
                actor.queue.remove(&Work::Value(completed_key.clone()));
                for id in job.waiters {
                    actor.finish(
                        id,
                        result
                            .clone()
                            .map(|valuation| Portfolio {
                                user: completed_key.user,
                                inventory: snapshot.inventory.clone(),
                                valuation,
                            })
                            .map_err(PortfolioError::Valuation),
                    );
                }
                actor.pump(ctx);
            }),
        );
        self.value = Some((key, handle));
    }
}
