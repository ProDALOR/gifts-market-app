//! Bounded work across peers; saved update progress advances only through completed deliveries.
use super::{
    BotError, ErrorDisposition, LaunchButton, MessageHandler, Reply,
    transport::{TelegramActions, Transport, UpdateCheckpoint},
};
use crate::{Event, Lane};
use futures_util::{
    FutureExt, StreamExt,
    future::BoxFuture,
    stream::{self, FuturesUnordered},
};
use grammers_session::updates::State;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    panic::AssertUnwindSafe,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

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
    event: Event,
    request: Option<R>,
    cancelled: CancellationToken,
}
struct Checkpoint {
    sequence: u64,
    state: State,
    done: bool,
}
struct Completed {
    sequence: u64,
    peer: Lane,
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
    let mut inline = HashMap::<u64, (u64, CancellationToken)>::new();
    let mut jobs = FuturesUnordered::<BoxFuture<'_, Completed>>::new();
    let mut sequence = 0u64;
    loop {
        while jobs.len() < limits.parallelism {
            let Some(index) = queued
                .iter()
                .position(|pending| !active.contains(&pending.event.lane()))
            else {
                break;
            };
            let pending = queued.remove(index).unwrap();
            let peer = pending.event.lane();
            active.insert(peer.clone());
            let actions = actions.clone();
            jobs.push(
                async move {
                    let work = process(
                        &actions,
                        handler,
                        pending.request,
                        pending.event,
                        url,
                        limits,
                    );
                    let result = tokio::select! { biased;
                        _=pending.cancelled.cancelled()=>Ok(()),
                        result=work=>result,
                    };
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
                if let Lane::InlineUser(user)=completed.peer
                    && inline.get(&user).is_some_and(|(seq,_)|*seq==completed.sequence) { inline.remove(&user); }
                checkpoints.iter_mut().find(|checkpoint|checkpoint.sequence==completed.sequence).unwrap().done=true;
            }
            Some(delivery)=deliveries.next(),if checkpoints.len()<limits.pending=>{
                let delivery=delivery?;
                let request=delivery.event.as_ref().and_then(|event|handler.parse_event(event,aliases));
                let responds=delivery.event.as_ref().is_some_and(|event|matches!(event,Event::Inline{..}|Event::Callback{..}));
                let done=request.is_none() && !responds;
                if !done {
                    let event=delivery.event.unwrap();
                    let cancelled=CancellationToken::new();
                    if let Some(user)=event.inline_user()
                        && let Some((_,old))=inline.insert(user,(sequence,cancelled.clone())) { old.cancel(); }
                    queued.push_back(Pending {sequence,event,request,cancelled});
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
    request: Option<H::Request>,
    event: Event,
    url: Option<&LaunchButton>,
    limits: Limits,
) -> Result<(), BotError> {
    actions.answer_callback(&event, request.is_some()).await?;
    // Inline queries arrive while typing; only evaluate the latest completed input.
    if matches!(event, Event::Inline { .. }) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let Some(request) = request else {
        return actions.send(&event, None, url).await;
    };
    let work = async move {
        let mut backoff = 1;
        for attempt in 0..limits.handler_attempts {
            match handler.handle(request.clone()).await {
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
            ..Default::default()
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
                ..Default::default()
            }
        }
        Err(_) => Reply {
            text: "This command took too long. Please try again shortly.".into(),
            ..Default::default()
        },
    }
    .bounded();
    let mut backoff = 1;
    loop {
        let result =
            tokio::time::timeout(limits.send_timeout, actions.send(&event, Some(&reply), url))
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
