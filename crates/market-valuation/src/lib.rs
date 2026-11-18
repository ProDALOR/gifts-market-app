//! Bounded valuation actor, independent of inventory, market storage and transports.
mod actor;
use std::{future::Future, hash::Hash, ops::Deref, sync::Arc, time::Duration};
use tokio::sync::{Semaphore, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    pub priced: u32,
    pub unpriced: u32,
}
/// Strategy output from a single consistent source snapshot. Currency belongs to `value`.
#[derive(Debug)]
pub struct Evaluation<V> {
    pub value: V,
    pub revision: String,
    pub coverage: Coverage,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Mark<V> {
    pub value: V,
    pub revision: String,
    pub evaluated_at: jiff::Timestamp,
    pub coverage: Coverage,
}
impl<V> Deref for Mark<V> {
    type Target = V;
    fn deref(&self) -> &V {
        &self.value
    }
}
impl<V> Mark<V> {
    pub fn evaluated(result: Evaluation<V>) -> Self {
        Self {
            value: result.value,
            revision: result.revision,
            coverage: result.coverage,
            evaluated_at: jiff::Timestamp::now(),
        }
    }
}
pub trait Strategy: Send + Sync + 'static {
    type Key: Clone + Eq + Hash + Send + 'static;
    type Value: Send + Sync + 'static;
    type Error: Clone + Send + 'static;
    fn normalize(&self, key: Self::Key) -> Result<Self::Key, Self::Error> {
        Ok(key)
    }
    /// A coalescing local invalidation signal, including source replacement and freshness changes.
    fn changes(&self) -> watch::Receiver<u64>;
    fn evaluate(
        &self,
        key: Self::Key,
    ) -> impl Future<Output = Result<Evaluation<Self::Value>, Self::Error>> + Send;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error<E> {
    Busy,
    Stopped,
    TimedOut,
    Strategy(E),
}
#[derive(Debug, Clone)]
pub struct Config {
    pub max_requests: usize,
    pub max_entries: usize,
    pub refresh_interval: Duration,
    pub idle_ttl: Duration,
    pub attempt_timeout: Duration,
    pub debounce: Duration,
    pub request_timeout: Duration,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            max_requests: 128,
            max_entries: 128,
            refresh_interval: Duration::from_secs(30),
            idle_ttl: Duration::from_secs(300),
            attempt_timeout: Duration::from_secs(10),
            debounce: Duration::from_millis(100),
            request_timeout: Duration::from_secs(30),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceError;
impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("valuation service unavailable or invalid configuration")
    }
}
impl std::error::Error for ServiceError {}
pub struct Handle<K, V, E> {
    tx: mpsc::Sender<actor::Request<K, V, E>>,
    slots: Arc<Semaphore>,
    closed: CancellationToken,
    stop: CancellationToken,
    wake: Arc<tokio::sync::Notify>,
    timeout: Duration,
}
impl<K, V, E> Clone for Handle<K, V, E> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            slots: self.slots.clone(),
            closed: self.closed.clone(),
            stop: self.stop.clone(),
            wake: self.wake.clone(),
            timeout: self.timeout,
        }
    }
}
impl<K: Send + 'static, V: Send + Sync + 'static, E: Send + 'static> Handle<K, V, E> {
    pub async fn mark(&self, key: K) -> Result<Arc<Mark<V>>, Error<E>> {
        if self.closed.is_cancelled() || self.stop.is_cancelled() {
            return Err(Error::Stopped);
        }
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let (reply, response) = oneshot::channel();
        self.tx
            .try_send(actor::Request {
                key,
                reply,
                _permit: permit,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => Error::Busy,
                _ => Error::Stopped,
            })?;
        let _wake = WakeOnDrop(self.wake.clone());
        tokio::select! {biased;_=self.stop.cancelled()=>Err(Error::Stopped),_=self.closed.cancelled()=>Err(Error::Stopped),result=tokio::time::timeout(self.timeout,response)=>match result{Ok(result)=>result.unwrap_or(Err(Error::Stopped)),Err(_)=>Err(Error::TimedOut)}}
    }
    pub async fn closed(&self) {
        self.closed.cancelled().await;
    }
}
pub struct Service<K, V, E> {
    pub handle: Handle<K, V, E>,
    pub task: Task,
}
pub struct Task {
    stop: CancellationToken,
    closed: CancellationToken,
    job: Option<tokio::task::JoinHandle<()>>,
}
impl<K: Clone + Eq + Hash + Send + 'static, V: Send + Sync + 'static, E: Clone + Send + 'static>
    Service<K, V, E>
{
    pub fn start<S: Strategy<Key = K, Value = V, Error = E>>(
        strategy: S,
        config: Config,
    ) -> Result<Self, ServiceError> {
        if config.max_requests == 0
            || config.max_requests > 4096
            || config.max_entries == 0
            || config.max_entries > 4096
            || config.refresh_interval.is_zero()
            || config.idle_ttl < config.refresh_interval
            || config.request_timeout < config.attempt_timeout
            || config.attempt_timeout.is_zero()
            || [
                config.idle_ttl,
                config.attempt_timeout,
                config.refresh_interval,
                config.debounce,
                config.request_timeout,
            ]
            .iter()
            .any(|d| *d > Duration::from_secs(3600))
        {
            return Err(ServiceError);
        }
        let (tx, rx) = mpsc::channel(config.max_requests);
        let stop = CancellationToken::new();
        let closed = CancellationToken::new();
        let guard = closed.clone().drop_guard();
        let wake = Arc::new(tokio::sync::Notify::new());
        let notify = wake.clone();
        let handle = Handle {
            tx,
            slots: Arc::new(Semaphore::new(config.max_requests)),
            closed: closed.clone(),
            stop: stop.clone(),
            wake,
            timeout: config.request_timeout,
        };
        let cancel = stop.clone();
        let job = tokio::spawn(async move {
            let _guard = guard;
            actor::run(strategy, config, rx, cancel, notify).await;
        });
        Ok(Self {
            handle,
            task: Task {
                stop,
                closed,
                job: Some(job),
            },
        })
    }
}
impl Task {
    async fn join(&mut self) -> Result<(), ServiceError> {
        if let Some(job) = self.job.take() {
            job.await.map_err(|_| ServiceError)?;
        }
        Ok(())
    }
    pub async fn shutdown(mut self) -> Result<(), ServiceError> {
        self.stop.cancel();
        self.join().await
    }
    pub async fn run(mut self, stop: CancellationToken) -> Result<(), ServiceError> {
        let expected = tokio::select! {biased;_=stop.cancelled()=>{self.stop.cancel();true},_=self.closed.cancelled()=>false};
        self.join().await?;
        if expected { Ok(()) } else { Err(ServiceError) }
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
#[cfg(test)]
mod tests;

struct WakeOnDrop(Arc<tokio::sync::Notify>);
impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}
