use crate::{
    InventoryProvider, Portfolio, PortfolioConfig, PortfolioError, PortfolioRequest,
    ValuationProvider, actor,
};
use actix::Recipient;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};
use tokio_util::sync::CancellationToken;

#[derive(actix::Message)]
#[rtype(result = "()")]
pub(crate) struct Submit {
    pub id: u64,
    pub request: PortfolioRequest,
    pub reply: oneshot::Sender<Result<Portfolio, PortfolioError>>,
    pub _permit: OwnedSemaphorePermit,
}
#[derive(actix::Message)]
#[rtype(result = "()")]
pub(crate) struct Cancel(pub u64);
#[derive(Clone)]
pub struct PortfolioHandle {
    submit: Recipient<Submit>,
    cancel: Recipient<Cancel>,
    slots: Arc<Semaphore>,
    sequence: Arc<AtomicU64>,
    stop: CancellationToken,
    closed: CancellationToken,
    timeout: std::time::Duration,
}
struct RequestGuard {
    id: u64,
    cancel: Recipient<Cancel>,
    armed: bool,
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancel.do_send(Cancel(self.id));
        }
    }
}
impl PortfolioHandle {
    pub(crate) fn new(
        submit: Recipient<Submit>,
        cancel: Recipient<Cancel>,
        config: &PortfolioConfig,
        stop: CancellationToken,
        closed: CancellationToken,
    ) -> Self {
        Self {
            submit,
            cancel,
            slots: Arc::new(Semaphore::new(config.max_requests)),
            sequence: Arc::new(AtomicU64::new(0)),
            stop,
            closed,
            timeout: config.request_timeout,
        }
    }
    /// For trusted application callers. The API/bot must derive the subject from verified identity.
    /// Dropping this future cancels this caller's interest without cancelling other callers.
    pub async fn request(&self, request: PortfolioRequest) -> Result<Portfolio, PortfolioError> {
        if self.stop.is_cancelled() || self.closed.is_cancelled() {
            return Err(PortfolioError::Stopped);
        }
        if !request.strategy.is_portfolio_strategy() {
            let mut details =
                market_gifts::valuation::ValuationErrorDetails::strategy(request.strategy);
            details.currency_id = Some(request.currency_id);
            return Err(PortfolioError::Valuation(
                market_valuation::ValuationError::with_details(
                    market_valuation::ValuationErrorCode::UnsupportedStrategy,
                    "Portfolio valuation requires model_floor or collection_floor",
                    details,
                ),
            ));
        }
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| PortfolioError::Busy)?;
        let id = self
            .sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| PortfolioError::Stopped)?;
        let (reply, response) = oneshot::channel();
        self.submit
            .try_send(Submit {
                id,
                request,
                reply,
                _permit: permit,
            })
            .map_err(|error| match error {
                actix::prelude::SendError::Full(_) => PortfolioError::Busy,
                actix::prelude::SendError::Closed(_) => PortfolioError::Stopped,
            })?;
        let mut guard = RequestGuard {
            id,
            cancel: self.cancel.clone(),
            armed: true,
        };
        let result = tokio::select! { biased;
            _ = self.stop.cancelled() => Err(PortfolioError::Stopped),
            _ = self.closed.cancelled() => Err(PortfolioError::Stopped),
            result = tokio::time::timeout(self.timeout, response) => match result {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(PortfolioError::Stopped),
                Err(_) => Err(PortfolioError::TimedOut),
            },
        };
        // Completed requests have already left the actor. Timeouts still need cancellation.
        guard.armed = matches!(result, Err(PortfolioError::TimedOut));
        drop(guard);
        result
    }
}
/// Owns an isolated Actix thread. No HTTP/Actix system is required in the caller.
pub struct PortfolioService {
    pub handle: PortfolioHandle,
    pub task: PortfolioTask,
}
pub struct PortfolioTask {
    stop: CancellationToken,
    closed: CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl PortfolioService {
    pub async fn start<P: InventoryProvider, V: ValuationProvider>(
        provider: P,
        valuation: V,
        config: PortfolioConfig,
        invocations: market_invocations::Handle,
    ) -> Result<Self, PortfolioError> {
        config.validate()?;
        let stop = CancellationToken::new();
        let closed = CancellationToken::new();
        let (ready, started) = oneshot::channel();
        let actor_stop = stop.clone();
        let actor_closed = closed.clone();
        let thread = std::thread::Builder::new()
            .name("market-portfolio".into())
            .spawn(move || {
                let _closed = actor_closed.clone().drop_guard();
                let system = actix::System::new();
                system.block_on(async move {
                    let handle = actor::start(
                        provider,
                        valuation,
                        config,
                        invocations,
                        actor_stop,
                        actor_closed.clone(),
                    );
                    if ready.send(handle).is_err() {
                        actix::System::current().stop();
                        return;
                    }
                    actix::spawn(async move {
                        actor_closed.cancelled().await;
                        actix::System::current().stop();
                    });
                });
                let _ = system.run();
            })
            .map_err(|_| PortfolioError::Stopped)?;
        let mut task = PortfolioTask {
            stop,
            closed,
            thread: Some(thread),
        };
        match started.await {
            Ok(handle) => Ok(Self { handle, task }),
            Err(_) => {
                task.stop.cancel();
                task.join().await?;
                Err(PortfolioError::Stopped)
            }
        }
    }
}
impl PortfolioTask {
    async fn join(&mut self) -> Result<(), PortfolioError> {
        if let Some(thread) = self.thread.take() {
            tokio::task::spawn_blocking(move || thread.join())
                .await
                .map_err(|_| PortfolioError::Stopped)?
                .map_err(|_| PortfolioError::Stopped)?;
        }
        Ok(())
    }
    pub async fn shutdown(mut self) -> Result<(), PortfolioError> {
        self.stop.cancel();
        self.closed.cancelled().await;
        self.join().await
    }
    /// Keep under application supervision; unexpected actor exit is reported as an error.
    pub async fn run(mut self, stop: CancellationToken) -> Result<(), PortfolioError> {
        let unexpected = tokio::select! { biased;
            _ = stop.cancelled() => { self.stop.cancel(); false },
            _ = self.closed.cancelled() => !self.stop.is_cancelled(),
        };
        self.closed.cancelled().await;
        self.join().await?;
        if unexpected {
            Err(PortfolioError::Stopped)
        } else {
            Ok(())
        }
    }
}
impl Drop for PortfolioTask {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
