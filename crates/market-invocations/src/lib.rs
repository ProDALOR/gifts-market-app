//! Bounded, typed I/O scheduling shared by services. No transport or domain dependencies.
mod scheduler;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    sync::{Semaphore, mpsc, oneshot},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Priority {
    Interactive,
    #[default]
    Normal,
    Background,
}
impl Priority {
    pub fn index(self) -> usize {
        self as usize
    }
}
/// Keys are transport-defined; include the authenticated account in `owner`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RateKey {
    pub owner: u64,
    pub group: &'static str,
    pub resource: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimit {
    pub key: RateKey,
    pub interval: Duration,
    pub concurrency: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replay {
    Safe,
    Deduplicated,
    Never,
}
#[derive(Debug, Clone)]
pub struct Policy {
    pub limits: Vec<RateLimit>,
    pub replay: Replay,
    pub attempt_timeout: Duration,
    pub max_attempts: u32,
}
#[derive(Debug, Clone)]
pub enum Retry {
    Never,
    Transient,
    After { delay: Duration, scope: RateKey },
}
#[derive(Debug)]
pub struct AttemptError<E> {
    pub error: E,
    pub retry: Retry,
}
/// Each execution is one bounded operation, not a pagination workflow.
/// Replays must use identical mutation IDs. Cancellation drops the execution future.
pub trait Invocation: Send + Sync + 'static {
    type Output: Send + 'static;
    type Error: Send + 'static;
    fn policy(&self) -> Policy;
    /// Optional owner lifetime (for example a transport connection generation).
    fn cancellation(&self) -> Option<CancellationToken> {
        None
    }
    fn execute(
        &self,
    ) -> impl Future<Output = Result<Self::Output, AttemptError<Self::Error>>> + Send;
}
/// Map transport results to application data without adding a second queue or losing retry hints.
pub trait InvocationExt: Invocation + Sized {
    fn map<T, E, F, G>(self, output: F, error: G) -> impl Invocation<Output = T, Error = E>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: Fn(Self::Output) -> T + Send + Sync + 'static,
        G: Fn(Self::Error) -> E + Send + Sync + 'static,
    {
        Mapped {
            inner: self,
            output,
            error,
        }
    }
}
impl<I: Invocation> InvocationExt for I {}
struct Mapped<I, F, G> {
    inner: I,
    output: F,
    error: G,
}
impl<I, T, E, F, G> Invocation for Mapped<I, F, G>
where
    I: Invocation,
    T: Send + 'static,
    E: Send + 'static,
    F: Fn(I::Output) -> T + Send + Sync + 'static,
    G: Fn(I::Error) -> E + Send + Sync + 'static,
{
    type Output = T;
    type Error = E;
    fn policy(&self) -> Policy {
        self.inner.policy()
    }
    fn cancellation(&self) -> Option<CancellationToken> {
        self.inner.cancellation()
    }
    async fn execute(&self) -> Result<T, AttemptError<E>> {
        self.inner
            .execute()
            .await
            .map(&self.output)
            .map_err(|e| AttemptError {
                error: (self.error)(e.error),
                retry: e.retry,
            })
    }
}
/// Build a typed operation without a bespoke wrapper type.
pub fn operation<T, E, F, Fut>(policy: Policy, call: F) -> impl Invocation<Output = T, Error = E>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, AttemptError<E>>> + Send,
{
    Function { policy, call }
}
struct Function<F> {
    policy: Policy,
    call: F,
}
impl<T, E, F, Fut> Invocation for Function<F>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, AttemptError<E>>> + Send,
{
    type Output = T;
    type Error = E;
    fn policy(&self) -> Policy {
        self.policy.clone()
    }
    async fn execute(&self) -> Result<T, AttemptError<E>> {
        (self.call)().await
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    Busy,
    Stopped,
    Cancelled,
    Deadline { retry_after: Option<Duration> },
    AttemptTimeout,
    InvalidPolicy,
}
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invocation {self:?}")
    }
}
impl std::error::Error for RuntimeError {}
#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    Runtime(RuntimeError),
    Operation(E),
}
#[derive(Debug, Clone)]
pub struct Options {
    pub priority: Priority,
    pub timeout: Duration,
    pub cancellation: CancellationToken,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            priority: Priority::Normal,
            timeout: Duration::from_secs(30),
            cancellation: CancellationToken::new(),
        }
    }
}
#[derive(Debug, Clone)]
pub struct Config {
    pub max_pending: usize,
    pub max_executing: usize,
    pub max_scopes: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            max_pending: 256,
            max_executing: 8,
            max_scopes: 2048,
        }
    }
}
#[derive(Clone)]
pub struct Handle {
    tx: mpsc::Sender<scheduler::Request>,
    slots: Arc<Semaphore>,
    stop: CancellationToken,
    changed: Arc<tokio::sync::Notify>,
}
pub struct Runtime {
    handle: Handle,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Runtime {
    /// Start once per application/account pool, outside HTTP workers and reconnect loops.
    pub fn start(config: Config) -> Result<Self, RuntimeError> {
        if config.max_pending == 0
            || config.max_pending > 65536
            || config.max_executing == 0
            || config.max_executing > config.max_pending
            || config.max_scopes == 0
            || config.max_scopes > 65536
        {
            return Err(RuntimeError::InvalidPolicy);
        }
        let (tx, rx) = mpsc::channel(config.max_pending);
        let handle = Handle {
            tx,
            slots: Arc::new(Semaphore::new(config.max_pending)),
            stop: CancellationToken::new(),
            changed: Arc::new(tokio::sync::Notify::new()),
        };
        let stop = handle.stop.clone();
        let changed = handle.changed.clone();
        // Capture the guard before spawning so even an unpolled aborted task closes consumers.
        let closed = stop.clone().drop_guard();
        let task = tokio::spawn(async move {
            let _closed = closed;
            scheduler::run(rx, stop, changed, config).await;
        });
        Ok(Self {
            handle,
            task: Some(task),
        })
    }
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
    pub async fn shutdown(mut self) {
        self.handle.stop.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.handle.stop.cancel();
    }
}
#[derive(Clone)]
pub struct Control {
    priority: Arc<AtomicU8>,
    changed: Arc<tokio::sync::Notify>,
}
impl Control {
    pub fn set_priority(&self, priority: Priority) {
        self.priority.store(priority as u8, Ordering::Relaxed);
        self.changed.notify_one();
    }
}
type InvocationFuture<T, E> = Pin<Box<dyn Future<Output = Result<T, Error<E>>> + Send>>;
pub struct Ticket<T, E> {
    future: Option<InvocationFuture<T, E>>,
    control: Control,
}
impl<T, E> Ticket<T, E> {
    pub fn control(&self) -> Control {
        self.control.clone()
    }
}
impl<T, E> Drop for Ticket<T, E> {
    fn drop(&mut self) {
        self.future.take();
        self.control.changed.notify_one();
    }
}
impl<T, E> Future for Ticket<T, E> {
    type Output = Result<T, Error<E>>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.future.as_mut().unwrap().as_mut().poll(cx)
    }
}
impl Handle {
    pub fn is_closed(&self) -> bool {
        self.stop.is_cancelled()
    }
    /// Observe explicit shutdown or unexpected scheduler exit without cancelling other consumers.
    pub async fn closed(&self) {
        self.stop.cancelled().await;
    }
    pub fn submit<I: Invocation>(
        &self,
        operation: I,
        options: Options,
    ) -> Result<Ticket<I::Output, I::Error>, RuntimeError> {
        if self.stop.is_cancelled() {
            return Err(RuntimeError::Stopped);
        }
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| RuntimeError::Busy)?;
        let policy = operation.policy();
        let owner_stop = operation.cancellation().unwrap_or_default();
        if Instant::now().checked_add(policy.attempt_timeout).is_none()
            || policy.attempt_timeout.is_zero()
            || options.timeout.is_zero()
            || policy.max_attempts == 0
            || policy.max_attempts > 100
            || policy.limits.len() > 16
            || policy.limits.iter().enumerate().any(|(i, l)| {
                l.concurrency == 0
                    || Instant::now().checked_add(l.interval).is_none()
                    || policy.limits[..i].iter().any(|old| old.key == l.key)
            })
        {
            return Err(RuntimeError::InvalidPolicy);
        }
        let deadline = Instant::now()
            .checked_add(options.timeout)
            .ok_or(RuntimeError::InvalidPolicy)?;
        let control = Control {
            priority: Arc::new(AtomicU8::new(options.priority as u8)),
            changed: self.changed.clone(),
        };
        let priority = control.priority.clone();
        let blocked = Arc::new(std::sync::Mutex::new(None::<Instant>));
        let handle = self.clone();
        let future = Box::pin(async move {
            let _slot = slot;
            let run = async {
                let mut attempts = 0u32;
                let mut earliest = Instant::now();
                loop {
                    if Instant::now() >= deadline {
                        return Err(Error::Runtime(RuntimeError::Deadline { retry_after: None }));
                    }
                    let (tx, rx) = oneshot::channel();
                    handle
                        .tx
                        .try_send(scheduler::Request {
                            limits: policy.limits.clone(),
                            priority: priority.clone(),
                            queued_priority: priority.load(Ordering::Relaxed),
                            earliest,
                            blocked: blocked.clone(),
                            reply: tx,
                        })
                        .map_err(|e| {
                            Error::Runtime(match e {
                                mpsc::error::TrySendError::Full(_) => RuntimeError::Busy,
                                mpsc::error::TrySendError::Closed(_) => RuntimeError::Stopped,
                            })
                        })?;
                    let permit = rx
                        .await
                        .map_err(|_| Error::Runtime(RuntimeError::Stopped))?
                        .map_err(Error::Runtime)?;
                    if Instant::now() >= deadline {
                        return Err(Error::Runtime(RuntimeError::Deadline { retry_after: None }));
                    }
                    *blocked.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    attempts = attempts.saturating_add(1);
                    let result =
                        tokio::time::timeout(policy.attempt_timeout, operation.execute()).await;
                    match result {
                        Ok(Ok(output)) => {
                            permit.finish(None);
                            return Ok(output);
                        }
                        Ok(Err(failure)) => {
                            let cooldown = match &failure.retry {
                                Retry::After { delay, scope } => Some((scope.clone(), *delay)),
                                _ => None,
                            };
                            if cooldown.as_ref().is_some_and(|(key, delay)| {
                                delay.is_zero()
                                    || !policy.limits.iter().any(|limit| &limit.key == key)
                                    || Instant::now().checked_add(*delay).is_none()
                            }) {
                                permit.finish(None);
                                return Err(Error::Runtime(RuntimeError::InvalidPolicy));
                            }
                            permit.finish(cooldown);
                            if policy.replay == Replay::Never {
                                return Err(Error::Operation(failure.error));
                            }
                            match failure.retry {
                                Retry::After { .. } => {}
                                Retry::Transient if attempts < policy.max_attempts => {
                                    earliest = Instant::now()
                                        + Duration::from_millis(250) * (1u32 << attempts.min(5));
                                }
                                _ => return Err(Error::Operation(failure.error)),
                            }
                        }
                        Err(_) => {
                            permit.finish(None);
                            return Err(Error::Runtime(RuntimeError::AttemptTimeout));
                        }
                    }
                }
            };
            tokio::select! {
                biased;
                _=handle.stop.cancelled()=>Err(Error::Runtime(RuntimeError::Stopped)),
                _=options.cancellation.cancelled()=>Err(Error::Runtime(RuntimeError::Cancelled)),
                _=owner_stop.cancelled()=>Err(Error::Runtime(RuntimeError::Cancelled)),
                result=tokio::time::timeout_at(deadline,run)=>result.unwrap_or_else(|_|Err(Error::Runtime(RuntimeError::Deadline{retry_after:blocked.lock().unwrap_or_else(|e|e.into_inner()).map(|t|t.saturating_duration_since(Instant::now())).filter(|d|!d.is_zero())}))),
            }
        });
        Ok(Ticket {
            future: Some(future),
            control,
        })
    }
}
#[cfg(test)]
mod tests;
