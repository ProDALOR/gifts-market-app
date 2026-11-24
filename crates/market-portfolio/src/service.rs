use crate::*;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot, watch};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Copy)]
pub(crate) enum Action {
    Query,
    Watch,
    Refresh,
}
#[derive(actix::Message)]
#[rtype(result = "()")]
pub(crate) struct Request {
    pub query: PortfolioQuery,
    pub action: Action,
    pub reply: oneshot::Sender<Result<watch::Receiver<Arc<PortfolioState>>, PortfolioError>>,
}
#[derive(Clone)]
pub struct PortfolioHandle {
    pub(crate) address: actix::Recipient<Request>,
    pub(crate) slots: Arc<Semaphore>,
    pub(crate) stop: CancellationToken,
    pub(crate) closed: CancellationToken,
}
pub struct PortfolioWatch {
    receiver: watch::Receiver<Arc<PortfolioState>>,
    stop: CancellationToken,
    closed: CancellationToken,
    _permit: OwnedSemaphorePermit,
}
impl PortfolioHandle {
    async fn invoke(
        &self,
        query: PortfolioQuery,
        action: Action,
    ) -> Result<watch::Receiver<Arc<PortfolioState>>, PortfolioError> {
        if self.stop.is_cancelled() || self.closed.is_cancelled() {
            return Err(PortfolioError::Stopped);
        }
        let (reply, receive) = oneshot::channel();
        self.address
            .try_send(Request {
                query,
                action,
                reply,
            })
            .map_err(|e| match e {
                actix::prelude::SendError::Full(_) => PortfolioError::Busy,
                _ => PortfolioError::Stopped,
            })?;
        tokio::select! {biased;
            _=self.stop.cancelled()=>Err(PortfolioError::Stopped),
            _=self.closed.cancelled()=>Err(PortfolioError::Stopped),
            v=tokio::time::timeout(std::time::Duration::from_secs(5),receive)=>v.map_err(|_|PortfolioError::TimedOut)?.map_err(|_|PortfolioError::Stopped)?,
        }
    }
    pub async fn query(
        &self,
        query: PortfolioQuery,
    ) -> Result<Arc<PortfolioState>, PortfolioError> {
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| PortfolioError::Busy)?;
        Ok(self.invoke(query, Action::Query).await?.borrow().clone())
    }
    pub async fn refresh(
        &self,
        query: PortfolioQuery,
    ) -> Result<Arc<PortfolioState>, PortfolioError> {
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| PortfolioError::Busy)?;
        Ok(self.invoke(query, Action::Refresh).await?.borrow().clone())
    }
    pub async fn watch(&self, query: PortfolioQuery) -> Result<PortfolioWatch, PortfolioError> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| PortfolioError::Busy)?;
        Ok(PortfolioWatch {
            receiver: self.invoke(query, Action::Watch).await?,
            stop: self.stop.clone(),
            closed: self.closed.clone(),
            _permit: permit,
        })
    }
}
impl PortfolioWatch {
    pub fn current(&mut self) -> Arc<PortfolioState> {
        self.receiver.borrow_and_update().clone()
    }
    pub async fn changed(&mut self) -> Result<Arc<PortfolioState>, PortfolioError> {
        tokio::select! {biased;
            _=self.stop.cancelled()=>return Err(PortfolioError::Stopped),
            _=self.closed.cancelled()=>return Err(PortfolioError::Stopped),
            v=self.receiver.changed()=>v.map_err(|_|PortfolioError::Stopped)?,
        }
        Ok(self.current())
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
