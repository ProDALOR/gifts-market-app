//! Application RPC policy. Grammers retains protocol retransmission, DC migration and update catch-up.
use super::client::rpc;
use crate::BotError;
use grammers_client::{Client, sender::InvocationError, tl};
use market_invocations::{
    AttemptError, Error, Handle, Invocation, Options, Policy, Priority, RateKey, RateLimit, Replay,
    Retry, RuntimeError,
};
use std::{future::Future, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
pub(super) enum Group {
    Control,
    Gifts,
    Media,
    Reply(i64),
}
#[derive(Clone)]
pub(super) struct Executor {
    pub handle: Handle,
    pub account: u64,
    pub stopped: CancellationToken,
}
impl Executor {
    fn account_key(&self) -> RateKey {
        RateKey {
            owner: self.account,
            group: "account",
            resource: None,
        }
    }
    fn policy(&self, group: Group, replay: Replay) -> Policy {
        let (name, interval, concurrency) = match group {
            Group::Control => ("control", 100, 1),
            Group::Gifts => ("gifts", 300, 1),
            Group::Media => ("media", 100, 2),
            Group::Reply(_) => ("replies", 50, 4),
        };
        let mut limits = vec![
            RateLimit {
                key: self.account_key(),
                interval: Duration::ZERO,
                concurrency: 8,
            },
            RateLimit {
                key: RateKey {
                    owner: self.account,
                    group: name,
                    resource: None,
                },
                interval: Duration::from_millis(interval),
                concurrency,
            },
        ];
        if let Group::Reply(peer) = group {
            limits.push(RateLimit {
                key: RateKey {
                    owner: self.account,
                    group: "reply-peer",
                    resource: Some(peer),
                },
                interval: Duration::from_secs(1),
                concurrency: 1,
            });
        }
        Policy {
            limits,
            replay,
            attempt_timeout: Duration::from_secs(10),
            max_attempts: 3,
        }
    }
    pub fn operation<T, F, Fut>(
        &self,
        group: Group,
        replay: Replay,
        call: F,
    ) -> impl Invocation<Output = T, Error = InvocationError> + use<T, F, Fut>
    where
        T: Send + 'static,
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, InvocationError>> + Send,
    {
        let scope = match group {
            Group::Reply(peer) => RateKey {
                owner: self.account,
                group: "reply-peer",
                resource: Some(peer),
            },
            _ => self.account_key(),
        };
        Call {
            stopped: self.stopped.clone(),
            policy: self.policy(group, replay),
            account: self.account_key(),
            slow_scope: scope,
            call,
        }
    }
    pub async fn run<T, F, Fut>(
        &self,
        group: Group,
        priority: Priority,
        replay: Replay,
        call: F,
    ) -> Result<T, CallError>
    where
        T: Send + 'static,
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, InvocationError>> + Send,
    {
        self.handle
            .submit(
                self.operation(group, replay, call),
                Options {
                    priority,
                    timeout: Duration::from_secs(30),
                    cancellation: self.stopped.clone(),
                },
            )
            .map_err(CallError::Runtime)?
            .await
            .map_err(|e| match e {
                Error::Runtime(e) => CallError::Runtime(e),
                Error::Operation(e) => CallError::Remote(e),
            })
    }
    pub async fn rpc<R>(
        &self,
        client: &Client,
        request: R,
        group: Group,
        priority: Priority,
        replay: Replay,
    ) -> Result<R::Return, BotError>
    where
        R: tl::RemoteCall + Send + Sync + 'static,
        R::Return: Send + 'static,
    {
        let request = Arc::new(request);
        let client = client.clone();
        self.run(group, priority, replay, move || {
            let request = request.clone();
            let client = client.clone();
            async move { client.invoke(&*request).await }
        })
        .await
        .map_err(CallError::bot)
    }
}
struct Call<F> {
    stopped: CancellationToken,
    policy: Policy,
    account: RateKey,
    slow_scope: RateKey,
    call: F,
}
impl<T, F, Fut> Invocation for Call<F>
where
    T: Send + 'static,
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, InvocationError>> + Send,
{
    type Output = T;
    type Error = InvocationError;
    fn policy(&self) -> Policy {
        self.policy.clone()
    }
    fn cancellation(&self) -> Option<CancellationToken> {
        Some(self.stopped.clone())
    }
    async fn execute(&self) -> Result<T, AttemptError<InvocationError>> {
        (self.call)().await.map_err(|error| {
            let retry = match &error {
                InvocationError::Rpc(e) if e.code == 420 => Retry::After {
                    delay: Duration::from_secs(e.value.unwrap_or(1).max(1) as u64),
                    scope: if e.name == "SLOWMODE_WAIT" {
                        self.slow_scope.clone()
                    } else {
                        self.account.clone()
                    },
                },
                InvocationError::Rpc(e) if e.code >= 500 => Retry::Transient,
                InvocationError::Io(_) => Retry::Transient,
                _ => Retry::Never,
            };
            AttemptError { error, retry }
        })
    }
}
pub(super) enum CallError {
    Remote(InvocationError),
    Runtime(RuntimeError),
}
impl CallError {
    pub fn bot(self) -> BotError {
        match self {
            Self::Remote(e) => rpc(e),
            Self::Runtime(RuntimeError::Stopped) => BotError::Invocations,
            Self::Runtime(RuntimeError::Cancelled) => BotError::Runner,
            Self::Runtime(RuntimeError::Deadline {
                retry_after: Some(delay),
            }) => BotError::RetryAfter(
                delay
                    .as_secs()
                    .saturating_add(u64::from(delay.subsec_nanos() > 0))
                    .max(1),
            ),
            Self::Runtime(_) => BotError::Transport,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_invocations::InvocationExt;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    fn error(code: i32, name: &str) -> InvocationError {
        InvocationError::Rpc(
            tl::types::RpcError {
                error_code: code,
                error_message: name.into(),
            }
            .into(),
        )
    }
    #[tokio::test]
    async fn classifies_rate_scopes_permanent_rejections_and_owner_lifetime() {
        let runtime = market_invocations::Runtime::start(Default::default()).unwrap();
        let stopped = CancellationToken::new();
        let executor = Executor {
            handle: runtime.handle(),
            account: 42,
            stopped: stopped.clone(),
        };
        for (code, name, scope) in [
            (420, "FLOOD_WAIT_7200", "account"),
            (420, "FLOOD_PREMIUM_WAIT_30", "account"),
            (420, "SLOWMODE_WAIT_60", "reply-peer"),
        ] {
            let call =
                executor.operation(Group::Reply(7), Replay::Deduplicated, move || async move {
                    Err::<(), _>(error(code, name))
                });
            let failure = call.execute().await.unwrap_err();
            let Retry::After {
                delay,
                scope: actual,
            } = failure.retry
            else {
                panic!("missing rate wait")
            };
            assert!(delay >= Duration::from_secs(30));
            assert_eq!(actual.owner, 42);
            assert_eq!(actual.group, scope);
            if scope == "reply-peer" {
                assert_eq!(actual.resource, Some(7));
            }
        }
        let call = executor.operation(Group::Gifts, Replay::Safe, || async {
            Err::<(), _>(error(403, "PRIVATE"))
        });
        assert!(matches!(
            call.execute().await.unwrap_err().retry,
            Retry::Never
        ));
        let called = Arc::new(AtomicUsize::new(0));
        let count = called.clone();
        let operation = executor.operation(Group::Gifts, Replay::Safe, move || {
            count.fetch_add(1, Ordering::Relaxed);
            async { Ok(()) }
        });
        let operation = operation.map(|value| value, |error| error);
        let ticket = executor
            .handle
            .submit(operation, Options::default())
            .unwrap();
        stopped.cancel();
        assert!(matches!(
            ticket.await,
            Err(Error::Runtime(RuntimeError::Cancelled))
        ));
        assert_eq!(called.load(Ordering::Relaxed), 0);
        assert_eq!(
            CallError::Runtime(RuntimeError::Stopped).bot(),
            BotError::Invocations
        );
        runtime.shutdown().await;
    }
    #[tokio::test(start_paused = true)]
    async fn scheduled_message_replays_keep_identical_random_ids_and_auth_does_not_replay() {
        let runtime = market_invocations::Runtime::start(Default::default()).unwrap();
        let executor = Executor {
            handle: runtime.handle(),
            account: 42,
            stopped: CancellationToken::new(),
        };
        let request = super::super::requests::reply_request(
            42,
            &crate::tests::message(1, "/test"),
            &crate::Reply {
                text: "hello".into(),
            },
            None,
        );
        let expected = request.random_id;
        let ids = Arc::new(Mutex::new(Vec::new()));
        let seen = ids.clone();
        executor
            .run(
                Group::Reply(7),
                Priority::Interactive,
                Replay::Deduplicated,
                move || {
                    let mut ids = seen.lock().unwrap();
                    ids.push(request.random_id);
                    let first = ids.len() == 1;
                    async move {
                        if first {
                            Err(error(500, "INTERNAL"))
                        } else {
                            Ok(())
                        }
                    }
                },
            )
            .await
            .unwrap_or_else(|_| panic!("replay failed"));
        assert_eq!(*ids.lock().unwrap(), [expected, expected]);
        let count = Arc::new(AtomicUsize::new(0));
        let called = count.clone();
        assert!(
            executor
                .run(
                    Group::Control,
                    Priority::Interactive,
                    Replay::Never,
                    move || {
                        called.fetch_add(1, Ordering::Relaxed);
                        async { Err::<(), _>(error(500, "INTERNAL")) }
                    }
                )
                .await
                .is_err()
        );
        assert_eq!(count.load(Ordering::Relaxed), 1);
        runtime.shutdown().await;
    }
}
