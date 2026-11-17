//! Bounded work across peers; saved update progress advances only through completed deliveries.
use super::{
    BotError, LaunchButton, MessageHandler, Reply,
    error::ErrorDisposition,
    transport::{Message, TelegramActions, Transport, UpdateCheckpoint, incoming},
};
use futures_util::{
    FutureExt, StreamExt,
    future::BoxFuture,
    stream::{self, FuturesUnordered},
};
use grammers_session::types::PeerId;
use grammers_session::updates::State;
use std::{
    collections::{HashSet, VecDeque},
    panic::AssertUnwindSafe,
    time::Duration,
};

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub parallelism: usize,
    pub pending: usize,
    pub handler_timeout: Duration,
    pub handler_attempts: usize,
    pub send_timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            parallelism: 8,
            pending: 64,
            handler_timeout: Duration::from_secs(40),
            handler_attempts: 3,
            send_timeout: Duration::from_secs(15),
        }
    }
}
struct Pending<R> {
    sequence: u64,
    message: Message,
    request: R,
}
struct Checkpoint {
    sequence: u64,
    state: State,
    done: bool,
}
struct Completed {
    sequence: u64,
    peer: PeerId,
    result: Result<(), BotError>,
}
pub(super) async fn run<T: Transport, H: MessageHandler>(
    transport: &mut T,
    handler: &H,
    url: Option<&LaunchButton>,
    aliases: &[String],
    limits: Limits,
) -> Result<(), BotError> {
    let actions = transport.actions();
    // Unfold retains an in-progress next() future when a command completes. Dropping
    // that future between receiving an update and resolving its peer could lose it.
    let mut deliveries = stream::unfold(transport, |transport| async move {
        let mut backoff = 1;
        let result = loop {
            match transport.next().await {
                Ok(delivery) => break Ok(delivery),
                Err(error) => {
                    if let Err(error) = retry(error, &mut backoff).await {
                        break Err(error);
                    }
                }
            }
        };
        Some((result, transport))
    })
    .boxed();
    let mut queued = VecDeque::<Pending<H::Request>>::new();
    let mut checkpoints = VecDeque::<Checkpoint>::new();
    let mut active = HashSet::new();
    let mut jobs = FuturesUnordered::<BoxFuture<'_, Completed>>::new();
    let mut sequence = 0u64;
    loop {
        while jobs.len() < limits.parallelism {
            let Some(index) = queued
                .iter()
                .position(|pending| !active.contains(&pending.message.peer.id))
            else {
                break;
            };
            let pending = queued.remove(index).unwrap();
            let peer = pending.message.peer.id;
            active.insert(peer);
            let actions = actions.clone();
            jobs.push(
                async move {
                    let result = process(
                        &actions,
                        handler,
                        pending.request,
                        pending.message,
                        url,
                        limits,
                    )
                    .await;
                    Completed {
                        sequence: pending.sequence,
                        peer,
                        result,
                    }
                }
                .boxed(),
            );
        }
        while checkpoints
            .front()
            .is_some_and(|checkpoint| checkpoint.done)
        {
            let checkpoint = checkpoints.pop_front().unwrap();
            actions.acknowledge(checkpoint.state).await?;
        }
        tokio::select! {
            Some(completed)=jobs.next(),if !jobs.is_empty()=>{
                completed.result?;
                active.remove(&completed.peer);
                checkpoints.iter_mut().find(|checkpoint|checkpoint.sequence==completed.sequence).unwrap().done=true;
            }
            Some(delivery)=deliveries.next(),if checkpoints.len()<limits.pending=>{
                let delivery=delivery?;
                let request=delivery.message.as_ref().and_then(|message|handler.parse(incoming(message),aliases));
                let done=request.is_none();
                if let Some(request)=request {
                    queued.push_back(Pending {sequence,message:delivery.message.unwrap(),request});
                }
                checkpoints.push_back(Checkpoint {sequence,state:delivery.state,done});
                sequence=sequence.checked_add(1).ok_or(BotError::InvalidResponse)?;
            }
        }
    }
}
async fn process<A: TelegramActions, H: MessageHandler>(
    actions: &A,
    handler: &H,
    request: H::Request,
    message: Message,
    url: Option<&LaunchButton>,
    limits: Limits,
) -> Result<(), BotError> {
    let gifts = actions.gift_source(&message);
    let work = async move {
        let mut backoff = 1;
        for attempt in 0..limits.handler_attempts {
            match handler.handle(request.clone(), &gifts).await {
                Ok(reply) => return Ok(reply),
                Err(error) if error.fatal() => return Err(error),
                Err(error)
                    if error.disposition() == ErrorDisposition::Retryable
                        && attempt + 1 < limits.handler_attempts =>
                {
                    retry(error, &mut backoff).await?
                }
                Err(_) => break,
            }
        }
        Ok(Reply {
            text: "This command could not be completed. Please try again shortly.".into(),
        })
    };
    let reply = match tokio::time::timeout(
        limits.handler_timeout,
        AssertUnwindSafe(work).catch_unwind(),
    )
    .await
    {
        Ok(Ok(result)) => result?,
        Ok(Err(_)) => {
            tracing::error!("bot command handler panicked");
            Reply {
                text: "This command could not be completed. Please try again shortly.".into(),
            }
        }
        Err(_) => Reply {
            text: "This command took too long. Please try again shortly.".into(),
        },
    }
    .bounded();
    let mut backoff = 1;
    loop {
        let result = tokio::time::timeout(limits.send_timeout, actions.send(&message, &reply, url))
            .await
            .unwrap_or(Err(BotError::Transport));
        match result {
            Ok(()) => return Ok(()),
            Err(error) if error.disposition() == ErrorDisposition::Rejected => {
                tracing::warn!(%error,"bot reply rejected; completing this delivery without a reply");
                return Ok(());
            }
            Err(error) => retry(error, &mut backoff).await?,
        }
    }
}
pub(super) async fn retry(error: BotError, backoff: &mut u64) -> Result<(), BotError> {
    if error.disposition() != ErrorDisposition::Retryable {
        return Err(error);
    }
    tracing::warn!(%error,"bot request failed; retrying");
    tokio::time::sleep(error.delay(*backoff)).await;
    *backoff = (*backoff * 2).min(30);
    Ok(())
}
