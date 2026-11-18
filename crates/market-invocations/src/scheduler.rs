use super::*;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::atomic::AtomicU8,
};
pub(crate) struct Request {
    pub limits: Vec<RateLimit>,
    pub priority: Arc<AtomicU8>,
    pub queued_priority: u8,
    pub earliest: Instant,
    pub blocked: Arc<std::sync::Mutex<Option<Instant>>>,
    pub reply: oneshot::Sender<Result<Permit, RuntimeError>>,
}
struct Complete {
    keys: Vec<RateKey>,
    cooldown: Option<(RateKey, Duration)>,
}
pub(crate) struct Permit {
    tx: mpsc::UnboundedSender<Complete>,
    keys: Option<Vec<RateKey>>,
}
impl Permit {
    pub fn finish(mut self, cooldown: Option<(RateKey, Duration)>) {
        let _ = self.tx.send(Complete {
            keys: self.keys.take().unwrap(),
            cooldown,
        });
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Some(keys) = self.keys.take() {
            let _ = self.tx.send(Complete {
                keys,
                cooldown: None,
            });
        }
    }
}
struct Scope {
    limit: RateLimit,
    active: usize,
    next: Instant,
}
pub(crate) async fn run(
    mut rx: mpsc::Receiver<Request>,
    stop: CancellationToken,
    changed: Arc<tokio::sync::Notify>,
    config: Config,
) {
    let (tx, mut completed) = mpsc::unbounded_channel::<Complete>();
    let mut queue = VecDeque::<Request>::new();
    let mut scopes = HashMap::<RateKey, Scope>::new();
    let mut active = 0usize;
    let mut credits = [4usize, 2, 1];
    loop {
        queue.retain(|r| !r.reply.is_closed());
        // Enter a new priority class at its tail, preserving FIFO within that class.
        let mut moved = Vec::new();
        for _ in 0..queue.len() {
            let mut request = queue.pop_front().unwrap();
            let priority = request.priority.load(Ordering::Relaxed);
            if priority != request.queued_priority {
                request.queued_priority = priority;
                moved.push(request);
            } else {
                queue.push_back(request);
            }
        }
        queue.extend(moved);
        let now = Instant::now();
        // Never evict live cooldowns, active scopes or keys referenced by queued work.
        while let Ok(completion) = completed.try_recv() {
            complete(completion, &mut scopes, &mut active);
        }
        for r in &queue {
            *r.blocked.lock().unwrap_or_else(|e| e.into_inner()) = r
                .limits
                .iter()
                .filter_map(|l| scopes.get(&l.key).map(|s: &Scope| s.next))
                .max();
        }
        let queued: HashSet<_> = queue
            .iter()
            .flat_map(|r| r.limits.iter().map(|l| l.key.clone()))
            .collect();
        scopes.retain(|key, s| s.active > 0 || s.next > now || queued.contains(key));
        while active < config.max_executing {
            let ready = |r: &Request| {
                r.earliest <= now
                    && r.limits.iter().all(|l| {
                        scopes
                            .get(&l.key)
                            .is_none_or(|s| s.active < l.concurrency && s.next <= now)
                    })
            };
            let mut found = None;
            for _ in 0..2 {
                for (p, credit) in credits.iter().enumerate() {
                    if *credit > 0
                        && let Some(i) = queue
                            .iter()
                            .position(|r| r.queued_priority as usize == p && ready(r))
                    {
                        found = Some((i, p));
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
                if !queue.iter().any(&ready) {
                    break;
                }
                credits = [4, 2, 1];
            }
            let Some((i, p)) = found else { break };
            let r = queue.remove(i).unwrap();
            let missing = r
                .limits
                .iter()
                .filter(|l| !scopes.contains_key(&l.key))
                .count();
            if scopes.len() + missing > config.max_scopes {
                let _ = r.reply.send(Err(RuntimeError::Busy));
                continue;
            }
            if r.limits
                .iter()
                .any(|l| scopes.get(&l.key).is_some_and(|s| s.limit != *l))
            {
                let _ = r.reply.send(Err(RuntimeError::InvalidPolicy));
                continue;
            }
            let mut keys = vec![];
            for limit in r.limits {
                let Some(next) = now.checked_add(limit.interval) else {
                    unreachable!("validated interval")
                };
                let state = scopes.entry(limit.key.clone()).or_insert_with(|| Scope {
                    limit: limit.clone(),
                    active: 0,
                    next: now,
                });
                state.active += 1;
                state.next = next;
                keys.push(limit.key);
            }
            active += 1;
            credits[p] -= 1;
            let _ = r.reply.send(Ok(Permit {
                tx: tx.clone(),
                keys: Some(keys),
            }));
        }
        let wake = if active < config.max_executing {
            queue
                .iter()
                .filter_map(|r| {
                    if r.limits.iter().any(|l| {
                        scopes
                            .get(&l.key)
                            .is_some_and(|s| s.active >= l.concurrency)
                    }) {
                        return None;
                    }
                    Some(
                        r.limits
                            .iter()
                            .filter_map(|l| scopes.get(&l.key).map(|s| s.next))
                            .fold(r.earliest, Instant::max),
                    )
                })
                .filter(|t| *t > now)
                .min()
        } else {
            None
        };
        tokio::select! {
            biased;
            _=stop.cancelled()=>break,
            Some(completion)=completed.recv()=>complete(completion,&mut scopes,&mut active),
            Some(request)=rx.recv()=>queue.push_back(request),
            _=changed.notified()=>{},
            _=async {match wake {Some(t)=>tokio::time::sleep_until(t).await,None=>std::future::pending().await}}=>{},
        }
    }
}

fn complete(done: Complete, scopes: &mut HashMap<RateKey, Scope>, active: &mut usize) {
    for key in done.keys {
        if let Some(s) = scopes.get_mut(&key) {
            s.active = s.active.saturating_sub(1);
        }
    }
    *active = active.saturating_sub(1);
    if let Some((key, delay)) = done.cooldown
        && let Some(s) = scopes.get_mut(&key)
        && let Some(until) = Instant::now().checked_add(delay)
    {
        s.next = s.next.max(until);
    }
}
