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
            handler_timeout: Duration::from_secs(90),
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
    let started = tokio::time::Instant::now();
    // Inline queries arrive while typing; only evaluate the latest completed input.
    if matches!(event, Event::Inline { .. }) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let Some(request) = request else {
        return actions.send(&event, None, url).await;
    };
    let (progress, mut updates) = tokio::sync::watch::channel(Reply::default());
    let work = async move {
        let mut backoff = 1;
        for attempt in 0..limits.handler_attempts {
            match handler
                .handle_with_progress(request.clone(), progress.clone())
                .await
            {
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
    let mut target = event.clone();
    let result = tokio::time::timeout(
        limits.handler_timeout,
        AssertUnwindSafe(work).catch_unwind(),
    );
    tokio::pin!(result);
    let mut pending = None;
    let mut open = true;
    let mut editable = true;
    let mut ticks = tokio::time::interval(Duration::from_secs(3));
    let completed = loop {
        tokio::select! {biased;
            result=&mut result=>break result,
            change=updates.changed(),if open=>{if change.is_err(){open=false;}else{pending=Some(updates.borrow_and_update().clone());}},
            _=ticks.tick(),if editable && pending.is_some()=>{
                let reply=pending.take().unwrap().bounded();
                match tokio::time::timeout(limits.send_timeout,actions.send_progress(&target,&reply,url)).await {
                    Ok(Ok(Some(next)))=>target=next,
                    _=>editable=false,
                }
            }
        }
    };
    let reply = match completed {
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
        let result = tokio::time::timeout(
            limits.send_timeout,
            actions.send(&target, Some(&reply), url),
        )
        .await
        .unwrap_or(Err(BotError::Transport));
        match result {
            Ok(()) => return Ok(()),
            Err(error) if error.disposition() == ErrorDisposition::Rejected => {
                tracing::warn!(
                    %error,
                    route = match &event { Event::Message(_) => "message", Event::Inline { .. } => "inline", Event::Guest { .. } => "guest", Event::Callback { .. } => "callback" },
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "bot reply rejected; completing this delivery without a reply"
                );
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

#[cfg(test)]
mod progress_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Actions(Arc<Mutex<Vec<(String, bool)>>>);
    impl TelegramActions for Actions {
        async fn identity(&self) -> Result<crate::BotUser, BotError> {
            Err(BotError::Transport)
        }
        async fn configure(
            &self,
            _: &[crate::MenuCommand],
            _: Option<&LaunchButton>,
        ) -> Result<(), BotError> {
            Ok(())
        }
        async fn send_progress(
            &self,
            event: &Event,
            reply: &Reply,
            _: Option<&LaunchButton>,
        ) -> Result<Option<Event>, BotError> {
            self.0
                .lock()
                .unwrap()
                .push((reply.text.clone(), matches!(event, Event::Callback { .. })));
            let mut message = crate::tests::message(99, "");
            message.outgoing = true;
            Ok(Some(Event::Callback {
                id: 0,
                user_id: 7,
                data: vec![],
                target: crate::CallbackTarget::Message(message),
            }))
        }
        async fn send(
            &self,
            event: &Event,
            reply: Option<&Reply>,
            _: Option<&LaunchButton>,
        ) -> Result<(), BotError> {
            self.0.lock().unwrap().push((
                reply.unwrap().text.clone(),
                matches!(event, Event::Callback { .. }),
            ));
            Ok(())
        }
    }
    struct Handler;
    impl MessageHandler for Handler {
        type Request = ();
        fn parse(&self, _: crate::IncomingMessage<'_>, _: &[String]) -> Option<()> {
            Some(())
        }
        async fn handle(&self, _: ()) -> Result<Reply, BotError> {
            Ok(Reply::text("done"))
        }
        async fn handle_with_progress(
            &self,
            _: (),
            updates: tokio::sync::watch::Sender<Reply>,
        ) -> Result<Reply, BotError> {
            updates.send_replace(Reply::text("page 1"));
            tokio::time::sleep(Duration::from_secs(4)).await;
            updates.send_replace(Reply::text("page 2"));
            tokio::time::sleep(Duration::from_secs(4)).await;
            Ok(Reply::text("done"))
        }
    }
    #[tokio::test(start_paused = true)]
    async fn progress_uses_one_message_then_edits_the_same_target_for_the_final_report() {
        let actions = Actions::default();
        process(
            &actions,
            &Handler,
            Some(()),
            Event::Message(crate::tests::message(1, "/portfolio")),
            None,
            Limits::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            *actions.0.lock().unwrap(),
            vec![
                ("page 1".into(), false),
                ("page 2".into(), true),
                ("done".into(), true)
            ]
        );
    }
}
