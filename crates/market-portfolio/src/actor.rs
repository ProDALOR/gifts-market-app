use crate::{
    service::{Action, Request},
    *,
};
use actix::prelude::*;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::{Semaphore, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
struct View {
    tx: watch::Sender<Arc<PortfolioState>>,
    report: Option<Arc<InventoryValuation>>,
    inventory: Option<Arc<InventorySnapshot>>,
    progress: usize,
    error: Option<String>,
    job: Option<SpawnHandle>,
    attempted: Option<Instant>,
    attempt: u64,
}
struct Entry {
    inventory: Option<Arc<InventorySnapshot>>,
    inventory_id: u64,
    refresh_id: u64,
    refreshing: bool,
    last_refresh: Option<Instant>,
    refresh_after: Option<jiff::Timestamp>,
    progress: PortfolioProgress,
    error: Option<String>,
    views: BTreeMap<market_gifts::CurrencyId, View>,
    used: u64,
}
struct PortfolioActor<P, V> {
    provider: Arc<P>,
    valuation: Arc<V>,
    config: PortfolioConfig,
    invocations: market_invocations::Handle,
    entries: BTreeMap<UserId, Entry>,
    clock: u64,
    next: u64,
    stop: CancellationToken,
    closed: CancellationToken,
    fetch_slots: Arc<Semaphore>,
}
#[derive(Message)]
#[rtype(result = "()")]
struct Progress {
    user: UserId,
    refresh: u64,
    checked: usize,
    pages: usize,
    total: Option<usize>,
}
#[derive(Message)]
#[rtype(result = "()")]
struct Valued {
    user: UserId,
    currency: market_gifts::CurrencyId,
    generation: u64,
    attempt: u64,
    count: usize,
}

pub(crate) fn start<P: InventoryProvider, V: ValuationProvider>(
    provider: P,
    valuation: V,
    config: PortfolioConfig,
    invocations: market_invocations::Handle,
    stop: CancellationToken,
    closed: CancellationToken,
) -> PortfolioHandle {
    let mut context = Context::new();
    context.set_mailbox_capacity(config.max_requests);
    let slots = Arc::new(Semaphore::new(config.max_requests));
    let actor = PortfolioActor {
        provider: Arc::new(provider),
        valuation: Arc::new(valuation),
        config,
        invocations,
        entries: BTreeMap::new(),
        clock: 0,
        next: 0,
        stop: stop.clone(),
        closed: closed.clone(),
        fetch_slots: Arc::new(Semaphore::new(2)),
    };
    let address = context.run(actor);
    PortfolioHandle {
        address: address.recipient(),
        slots,
        stop,
        closed,
    }
}
impl<P: InventoryProvider, V: ValuationProvider> Actor for PortfolioActor<P, V> {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.spawn(
            self.stop
                .clone()
                .cancelled_owned()
                .into_actor(self)
                .map(|_, _, ctx| ctx.stop()),
        );
        ctx.spawn(
            {
                let calls = self.invocations.clone();
                async move { calls.closed().await }
            }
            .into_actor(self)
            .map(|_, _, ctx| ctx.stop()),
        );
        ctx.run_interval(Duration::from_secs(1), |actor, ctx| {
            let active: Vec<_> = actor
                .entries
                .iter()
                .flat_map(|(user, e)| {
                    e.views
                        .iter()
                        .filter(|(_, v)| v.tx.receiver_count() > 0)
                        .map(|(c, _)| (*user, c.clone()))
                })
                .collect();
            for (user, currency) in active {
                actor.value(user, currency, ctx);
            }
        });
    }
    fn stopped(&mut self, _: &mut Self::Context) {
        self.closed.cancel();
    }
}
impl<P, V> Drop for PortfolioActor<P, V> {
    fn drop(&mut self) {
        self.closed.cancel();
    }
}
impl<P: InventoryProvider, V: ValuationProvider> Handler<Request> for PortfolioActor<P, V> {
    type Result = ();
    fn handle(&mut self, request: Request, ctx: &mut Context<Self>) {
        if request.reply.is_closed() {
            return;
        }
        let result = self.request(&request.query, request.action, ctx);
        let _ = request.reply.send(result);
    }
}
impl<P: InventoryProvider, V: ValuationProvider> Handler<Progress> for PortfolioActor<P, V> {
    type Result = ();
    fn handle(&mut self, p: Progress, _: &mut Context<Self>) {
        if let Some(e) = self.entries.get_mut(&p.user)
            && e.refreshing
            && e.refresh_id == p.refresh
        {
            e.progress.stage = PortfolioStage::LoadingInventory;
            e.progress.checked = p.checked;
            e.progress.pages = p.pages;
            e.progress.total = p.total;
            self.publish(p.user);
        }
    }
}
impl<P: InventoryProvider, V: ValuationProvider> Handler<Valued> for PortfolioActor<P, V> {
    type Result = ();
    fn handle(&mut self, p: Valued, _: &mut Context<Self>) {
        if let Some(e) = self.entries.get_mut(&p.user)
            && e.inventory_id == p.generation
            && !e.refreshing
            && let Some(v) = e.views.get_mut(&p.currency)
            && v.job.is_some()
            && v.attempt == p.attempt
        {
            v.progress = p.count;
            self.publish(p.user);
        }
    }
}
impl<P: InventoryProvider, V: ValuationProvider> PortfolioActor<P, V> {
    fn state(user: UserId, e: &Entry, v: &View) -> Arc<PortfolioState> {
        let mut progress = e.progress.clone();
        if !e.refreshing {
            progress.stage = if v.job.is_some() {
                PortfolioStage::Valuing
            } else if e.error.is_some() || v.error.is_some() {
                PortfolioStage::Failed
            } else if v.report.is_some() {
                PortfolioStage::Ready
            } else {
                PortfolioStage::Idle
            };
            progress.valued = v.progress;
            progress.collectibles = e.inventory.as_ref().map_or(0, |i| i.gifts.len());
        }
        let inventory = v.inventory.as_ref().map(|i| InventorySummary {
            checked_at: i.fetched_at,
            count: i.gifts.len(),
            scanned: i.gifts_scanned,
            regular: i.regular_gifts,
            skipped: i.skipped_count(),
            complete: i.completion == InventoryCompletion::Complete,
        });
        Arc::new(PortfolioState {
            user_id: user,
            refresh_id: e.refresh_id,
            progress,
            inventory,
            valuation: v.report.clone(),
            refresh_after: e.refresh_after,
            error: e.error.clone().or_else(|| v.error.clone()),
        })
    }
    fn publish(&self, user: UserId) {
        if let Some(e) = self.entries.get(&user) {
            for v in e.views.values() {
                let next = Self::state(user, e, v);
                v.tx.send_if_modified(|current| {
                    let same_report = match (&current.valuation, &next.valuation) {
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                        (None, None) => true,
                        _ => false,
                    };
                    if same_report
                        && current.refresh_id == next.refresh_id
                        && current.progress == next.progress
                        && current.inventory == next.inventory
                        && current.refresh_after == next.refresh_after
                        && current.error == next.error
                    {
                        return false;
                    }
                    *current = next;
                    true
                });
            }
        }
    }
    fn request(
        &mut self,
        q: &PortfolioQuery,
        action: Action,
        ctx: &mut Context<Self>,
    ) -> Result<watch::Receiver<Arc<PortfolioState>>, PortfolioError> {
        if self.stop.is_cancelled() {
            return Err(PortfolioError::Stopped);
        }
        if !self.entries.contains_key(&q.user_id) {
            if self.entries.len() >= self.config.cached_users {
                let oldest = self
                    .entries
                    .iter()
                    .filter(|(_, e)| {
                        !e.refreshing
                            && e.views
                                .values()
                                .all(|v| v.job.is_none() && v.tx.receiver_count() == 0)
                    })
                    .min_by_key(|(_, e)| e.used)
                    .map(|(u, _)| *u)
                    .ok_or(PortfolioError::Busy)?;
                self.entries.remove(&oldest);
            }
            self.entries.insert(
                q.user_id,
                Entry {
                    inventory: None,
                    inventory_id: 0,
                    refresh_id: 0,
                    refreshing: false,
                    last_refresh: None,
                    refresh_after: None,
                    progress: Default::default(),
                    error: None,
                    views: BTreeMap::new(),
                    used: 0,
                },
            );
        }
        self.clock = self.clock.saturating_add(1);
        let e = self.entries.get_mut(&q.user_id).unwrap();
        e.used = self.clock;
        if !e.views.contains_key(&q.currency_id) {
            if e.views.len() >= 8 {
                let old = e
                    .views
                    .iter()
                    .find(|(_, v)| v.job.is_none() && v.tx.receiver_count() == 0)
                    .map(|(c, _)| c.clone())
                    .ok_or(PortfolioError::Busy)?;
                e.views.remove(&old);
            }
            let empty = Arc::new(PortfolioState {
                user_id: q.user_id,
                refresh_id: e.refresh_id,
                progress: Default::default(),
                inventory: None,
                valuation: None,
                refresh_after: None,
                error: None,
            });
            let (tx, _) = watch::channel(empty);
            e.views.insert(
                q.currency_id.clone(),
                View {
                    tx,
                    report: None,
                    inventory: None,
                    progress: 0,
                    error: None,
                    job: None,
                    attempted: None,
                    attempt: 0,
                },
            );
        }
        let initial = matches!(action, Action::Watch) && e.last_refresh.is_none();
        let receiver = e.views[&q.currency_id].tx.subscribe();
        if matches!(action, Action::Refresh) || initial {
            self.refresh(q.user_id, ctx)?;
        }
        self.value(q.user_id, q.currency_id.clone(), ctx);
        self.publish(q.user_id);
        Ok(receiver)
    }
    fn refresh(&mut self, user: UserId, ctx: &mut Context<Self>) -> Result<(), PortfolioError> {
        let e = self.entries.get_mut(&user).unwrap();
        if e.refreshing {
            return Ok(());
        }
        if let Some(t) = e.last_refresh
            && t.elapsed() < self.config.refresh_interval
        {
            return Err(PortfolioError::Cooldown(
                self.config
                    .refresh_interval
                    .saturating_sub(t.elapsed())
                    .as_secs()
                    + 1,
            ));
        }
        self.next = self.next.checked_add(1).ok_or(PortfolioError::Busy)?;
        let refresh = self.next;
        e.refresh_id = refresh;
        e.refreshing = true;
        e.last_refresh = Some(Instant::now());
        e.refresh_after = Some(
            jiff::Timestamp::from_millisecond(
                jiff::Timestamp::now().as_millisecond()
                    + self.config.refresh_interval.as_millis() as i64,
            )
            .expect("bounded refresh interval"),
        );
        e.error = None;
        // Rechecking inventory supersedes its old valuation immediately, not only
        // after all Telegram pages arrive. Keep the last published report intact.
        for view in e.views.values_mut() {
            if let Some(job) = view.job.take() {
                ctx.cancel_future(job);
            }
        }
        e.progress = PortfolioProgress {
            stage: PortfolioStage::Queued,
            ..Default::default()
        };
        let provider = self.provider.clone();
        let calls = self.invocations.clone();
        let config = self.config.clone();
        let address = ctx.address();
        let stop = self.stop.clone();
        let slots = self.fetch_slots.clone();
        ctx.spawn(
            async move {
                let work = async {
                    let _permit = slots
                        .acquire_owned()
                        .await
                        .map_err(|_| PortfolioError::Stopped)?;
                    let mut builder = crate::inventory::InventoryBuilder::default();
                    loop {
                        let limit = config.page_size.min(config.max_gifts - builder.scanned);
                        let ticket = calls
                            .submit(
                                provider.page(InventoryPageRequest {
                                    user,
                                    cursor: builder.cursor.clone(),
                                    limit,
                                }),
                                market_invocations::Options {
                                    priority: market_invocations::Priority::Interactive,
                                    timeout: config.request_timeout,
                                    cancellation: stop.clone(),
                                },
                            )
                            .map_err(|_| PortfolioError::Busy)?;
                        let page = match ticket.await {
                            Ok(page) => page,
                            Err(market_invocations::Error::Operation(error)) => {
                                if builder.pages > 0
                                    && !matches!(
                                        error,
                                        InventoryError::Forbidden | InventoryError::Authorization
                                    )
                                {
                                    return Ok(builder.snapshot(InventoryCompletion::Partial(
                                        PartialReason::ReadFailed(error),
                                    )));
                                }
                                return Err(PortfolioError::Inventory(error));
                            }
                            Err(_) => return Err(PortfolioError::TimedOut),
                        };
                        let completion = builder
                            .merge(page, limit, &config)
                            .map_err(PortfolioError::Inventory)?;
                        address.do_send(Progress {
                            user,
                            refresh,
                            checked: builder.scanned,
                            pages: builder.pages,
                            total: builder.total,
                        });
                        if let Some(completion) = completion {
                            return Ok(builder.snapshot(completion));
                        }
                    }
                };
                tokio::time::timeout(config.request_timeout, work)
                    .await
                    .unwrap_or(Err(PortfolioError::TimedOut))
            }
            .into_actor(self)
            .map(move |result, actor, ctx| {
                let fits = result
                    .as_ref()
                    .is_ok_and(|i| actor.fit_inventory(user, i.gifts.len()));
                let e = actor.entries.get_mut(&user).unwrap();
                if e.refresh_id != refresh {
                    return;
                }
                e.refreshing = false;
                match result {
                    Ok(_) if !fits => {
                        e.error = Some(
                            "Portfolio cache capacity reached; previous inventory retained".into(),
                        );
                    }
                    Ok(inventory) => {
                        let complete = inventory.completion == InventoryCompletion::Complete;
                        if !complete
                            && e.inventory
                                .as_ref()
                                .is_some_and(|i| i.completion == InventoryCompletion::Complete)
                        {
                            e.error = Some(
                                "Refresh was partial; the previous complete inventory is retained"
                                    .into(),
                            );
                        } else {
                            e.inventory_id = refresh;
                            e.progress.checked = inventory.gifts_scanned;
                            e.progress.total = inventory.reported_total;
                            e.progress.pages = inventory.pages_read;
                            e.inventory = Some(Arc::new(inventory));
                            for view in e.views.values_mut() {
                                if let Some(job) = view.job.take() {
                                    ctx.cancel_future(job);
                                }
                                view.attempted = None;
                                view.progress = 0;
                            }
                        }
                    }
                    Err(error) => e.error = Some(error.to_string()),
                }
                let currencies: Vec<_> = e.views.keys().cloned().collect();
                for currency in currencies {
                    actor.value(user, currency, ctx);
                }
                actor.publish(user);
            }),
        );
        Ok(())
    }
    fn fit_inventory(&mut self, user: UserId, incoming: usize) -> bool {
        loop {
            let used: usize = self
                .entries
                .iter()
                .filter(|(u, _)| **u != user)
                .map(|(_, e)| e.inventory.as_ref().map_or(0, |i| i.gifts.len()))
                .sum();
            if used.saturating_add(incoming) <= 50000 {
                return true;
            }
            let old = self
                .entries
                .iter()
                .filter(|(u, e)| {
                    **u != user
                        && !e.refreshing
                        && e.views
                            .values()
                            .all(|v| v.job.is_none() && v.tx.receiver_count() == 0)
                })
                .min_by_key(|(_, e)| e.used)
                .map(|(u, _)| *u);
            if let Some(old) = old {
                self.entries.remove(&old);
            } else {
                return false;
            }
        }
    }
    fn value(&mut self, user: UserId, currency: market_gifts::CurrencyId, ctx: &mut Context<Self>) {
        let e = self.entries.get_mut(&user).unwrap();
        let Some(inventory) = &e.inventory else {
            return;
        };
        let v = e.views.get_mut(&currency).unwrap();
        if e.refreshing
            || v.job.is_some()
            || v.attempted
                .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
        {
            return;
        }
        v.attempted = Some(Instant::now());
        v.progress = 0;
        v.error = None;
        let valued_inventory = inventory.clone();
        let gifts = inventory.gifts.clone();
        v.attempt = v
            .attempt
            .checked_add(1)
            .expect("valuation attempt exhausted");
        let attempt = v.attempt;
        let generation = e.inventory_id;
        let provider = self.valuation.clone();
        let idle_timeout = self.config.valuation_idle_timeout;
        let (progress, mut received) = watch::channel(0usize);
        let addr = ctx.address();
        let progress_currency = currency.clone();
        let result_currency = currency.clone();
        let request_currency = currency.clone();
        let handle = ctx.spawn(
            async move {
                let work = provider.value(gifts, request_currency, progress);
                tokio::pin!(work);
                let stalled = tokio::time::sleep(idle_timeout);
                tokio::pin!(stalled);
                let mut completed = 0;
                let mut receiving = true;
                loop {
                    tokio::select! {
                        biased;
                        result = &mut work => break result,
                        update = received.changed(), if receiving => {
                            if update.is_err() {
                                receiving = false;
                                continue;
                            }
                            let count = *received.borrow_and_update();
                            if count > completed {
                                completed = count;
                                stalled.as_mut().reset(Instant::now() + idle_timeout);
                                addr.do_send(Valued { user, currency: progress_currency.clone(), generation, attempt, count });
                            }
                        }
                        _ = &mut stalled => break Err(market_valuation::ValuationError::timed_out(
                            "Portfolio valuation made no progress",
                        )),
                    }
                }
            }
            .into_actor(self)
            .map(move |result, actor, _| {
                let e = actor.entries.get_mut(&user).unwrap();
                if e.inventory_id != generation {
                    return;
                }
                let v = e.views.get_mut(&result_currency).unwrap();
                if v.attempt != attempt || e.refreshing {
                    return;
                }
                v.job = None;
                // A slow completed/stalled run must not immediately start another full pass.
                v.attempted = Some(Instant::now());
                match result {
                    Ok(report) => {
                        v.progress = report.items.len();
                        v.report = Some(report);
                        v.inventory = Some(valued_inventory);
                        v.error = None;
                    }
                    Err(error) => v.error = Some(error.to_string()),
                }
                actor.publish(user);
            }),
        );
        self.entries
            .get_mut(&user)
            .unwrap()
            .views
            .get_mut(&currency)
            .unwrap()
            .job = Some(handle);
    }
}
