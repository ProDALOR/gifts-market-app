//! Typed Actix requests, marks and lifecycle ownership. Actors own their scheduling and state.
use actix::{
    Actor, ActorContext, ActorFutureExt, AsyncContext, Context, Handler, Message, Recipient,
};
use std::{marker::PhantomData, ops::Deref, sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct Prepared<T> {
    pub data: T,
    pub revision: String,
    pub prepared_at: jiff::Timestamp,
}
impl<T> Prepared<T> {
    pub fn new(data: T, revision: String) -> Self {
        Self {
            data,
            revision,
            prepared_at: jiff::Timestamp::now(),
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct Mark<T> {
    pub strategy: &'static str,
    pub data: T,
    pub revision: String,
    pub evaluated_at: jiff::Timestamp,
}
impl<T> Deref for Mark<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.data
    }
}
impl<T> Mark<T> {
    pub fn evaluated(strategy: &'static str, value: Prepared<T>) -> Self {
        Self {
            strategy,
            data: value.data,
            revision: value.revision,
            evaluated_at: value.prepared_at,
        }
    }
    pub fn prepared<P>(strategy: &'static str, data: T, prepared: &Prepared<P>) -> Self {
        Self {
            strategy,
            data,
            revision: prepared.revision.clone(),
            evaluated_at: prepared.prepared_at,
        }
    }
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
    pub request_timeout: Duration,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            max_requests: 128,
            request_timeout: Duration::from_secs(30),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceError;
impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("valuation actor unavailable or invalid configuration")
    }
}
impl std::error::Error for ServiceError {}

/// The permit lives with the actor's response future, including after caller cancellation.
pub struct RequestContext {
    pub cancelled: CancellationToken,
    _permit: OwnedSemaphorePermit,
}
pub struct Evaluate<I, V, E> {
    input: I,
    context: RequestContext,
    _result: PhantomData<fn() -> (V, E)>,
}
impl<I, V, E> Evaluate<I, V, E> {
    pub fn into_parts(self) -> (I, RequestContext) {
        (self.input, self.context)
    }
}
impl<I, V: 'static, E: 'static> Message for Evaluate<I, V, E> {
    type Result = Result<Arc<Mark<V>>, E>;
}

pub struct Lifecycle {
    strategy: &'static str,
    stop: CancellationToken,
    closed: CancellationToken,
}
impl Lifecycle {
    pub fn strategy(&self) -> &'static str {
        self.strategy
    }
    pub fn stop_token(&self) -> CancellationToken {
        self.stop.clone()
    }
    pub fn started<A: Actor<Context = Context<A>>>(&self, ctx: &mut Context<A>) {
        let stop = self.stop.clone();
        ctx.spawn(
            actix::fut::wrap_future::<_, A>(async move { stop.cancelled().await })
                .map(|_, _, ctx| ctx.stop()),
        );
    }
}
impl Drop for Lifecycle {
    fn drop(&mut self) {
        self.closed.cancel();
    }
}
pub struct Handle<I: Send + 'static, V: Send + Sync + 'static, E: Send + 'static> {
    strategy: &'static str,
    recipient: Recipient<Evaluate<I, V, E>>,
    slots: Arc<Semaphore>,
    stop: CancellationToken,
    closed: CancellationToken,
    timeout: Duration,
}
impl<I: Send + 'static, V: Send + Sync + 'static, E: Send + 'static> Clone for Handle<I, V, E> {
    fn clone(&self) -> Self {
        Self {
            strategy: self.strategy,
            recipient: self.recipient.clone(),
            slots: self.slots.clone(),
            stop: self.stop.clone(),
            closed: self.closed.clone(),
            timeout: self.timeout,
        }
    }
}
impl<I: Send + 'static, V: Send + Sync + 'static, E: Send + 'static> Handle<I, V, E> {
    pub fn strategy(&self) -> &'static str {
        self.strategy
    }
    pub fn evaluate(
        &self,
        input: I,
    ) -> impl std::future::Future<Output = Result<Arc<Mark<V>>, Error<E>>> + Send + 'static {
        let handle = self.clone();
        async move {
            if handle.stop.is_cancelled() || handle.closed.is_cancelled() {
                return Err(Error::Stopped);
            }
            let permit = handle
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| Error::Busy)?;
            let cancelled = CancellationToken::new();
            let _cancel = cancelled.clone().drop_guard();
            let response = handle.recipient.send(Evaluate {
                input,
                context: RequestContext {
                    cancelled,
                    _permit: permit,
                },
                _result: PhantomData,
            });
            tokio::select! {
                biased;
                _ = handle.stop.clone().cancelled_owned() => Err(Error::Stopped),
                _ = handle.closed.clone().cancelled_owned() => Err(Error::Stopped),
                result = tokio::time::timeout(handle.timeout, response) => match result {
                    Ok(Ok(result)) => result.map_err(Error::Strategy),
                    Ok(Err(_)) => Err(Error::Stopped),
                    Err(_) => Err(Error::TimedOut),
                },
            }
        }
    }
    pub async fn closed(&self) {
        self.closed.cancelled().await;
    }
}
pub struct Service<I: Send + 'static, V: Send + Sync + 'static, E: Send + 'static> {
    pub handle: Handle<I, V, E>,
    pub task: Task,
}
pub struct Task {
    stop: CancellationToken,
    closed: CancellationToken,
}
impl<I: Send + 'static, V: Send + Sync + 'static, E: Send + 'static> Service<I, V, E> {
    pub async fn start_on<A, F>(
        data: &market_actors::MarketDataService,
        id: &'static str,
        build: F,
        config: Config,
    ) -> Result<Self, ServiceError>
    where
        A: Actor<Context = Context<A>> + Handler<Evaluate<I, V, E>>,
        F: FnOnce(market_actors::OrderbookReader, Lifecycle) -> A + Send + 'static,
    {
        data.start_actor(move |source| Self::start(id, config, |life| build(source, life)))
            .await
            .map_err(|_| ServiceError)?
    }
    pub fn start<A, F>(
        strategy: &'static str,
        config: Config,
        build: F,
    ) -> Result<Self, ServiceError>
    where
        A: Actor<Context = Context<A>> + Handler<Evaluate<I, V, E>>,
        F: FnOnce(Lifecycle) -> A,
    {
        if strategy.is_empty()
            || strategy.len() > 64
            || config.max_requests == 0
            || config.max_requests > 4096
            || config.request_timeout.is_zero()
            || config.request_timeout > Duration::from_secs(3600)
            || actix::System::try_current().is_none()
        {
            return Err(ServiceError);
        }
        let stop = CancellationToken::new();
        let closed = CancellationToken::new();
        let actor = build(Lifecycle {
            strategy,
            stop: stop.clone(),
            closed: closed.clone(),
        });
        let mut context = Context::<A>::new();
        context.set_mailbox_capacity(config.max_requests);
        let address = context.run(actor);
        Ok(Self {
            handle: Handle {
                strategy,
                recipient: address.recipient(),
                slots: Arc::new(Semaphore::new(config.max_requests)),
                stop: stop.clone(),
                closed: closed.clone(),
                timeout: config.request_timeout,
            },
            task: Task { stop, closed },
        })
    }
}
impl Task {
    pub async fn shutdown(self) -> Result<(), ServiceError> {
        self.stop.cancel();
        self.closed.cancelled().await;
        Ok(())
    }
    pub async fn run(self, stop: CancellationToken) -> Result<(), ServiceError> {
        let expected = tokio::select! {
            biased;
            _ = stop.cancelled() => { self.stop.cancel(); true },
            _ = self.closed.cancelled() => false,
        };
        self.closed.cancelled().await;
        if expected { Ok(()) } else { Err(ServiceError) }
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) fn test_lifecycle(
    strategy: &'static str,
) -> (Lifecycle, CancellationToken, CancellationToken) {
    let stop = CancellationToken::new();
    let closed = CancellationToken::new();
    (
        Lifecycle {
            strategy,
            stop: stop.clone(),
            closed: closed.clone(),
        },
        stop,
        closed,
    )
}
#[cfg(test)]
pub(crate) fn test_message<I, V, E>(input: I) -> Evaluate<I, V, E> {
    Evaluate {
        input,
        context: RequestContext {
            cancelled: CancellationToken::new(),
            _permit: Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
        },
        _result: PhantomData,
    }
}
