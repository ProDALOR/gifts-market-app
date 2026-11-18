use super::*;
use std::{collections::HashMap, pin::Pin};
use tokio::{sync::OwnedSemaphorePermit, time::Instant};
pub(crate) struct Request<K, V, E> {
    pub key: K,
    pub reply: oneshot::Sender<Result<Arc<Mark<V>>, Error<E>>>,
    pub _permit: OwnedSemaphorePermit,
}
struct Entry<K, V, E> {
    mark: Option<Arc<Mark<V>>>,
    dirty: bool,
    used: Instant,
    waiters: Vec<Request<K, V, E>>,
}
type Work<K, V, E> =
    Pin<Box<dyn Future<Output = (K, u64, Result<Evaluation<V>, Error<E>>)> + Send>>;
pub(crate) async fn run<S: Strategy>(
    strategy: S,
    config: Config,
    mut requests: mpsc::Receiver<Request<S::Key, S::Value, S::Error>>,
    stop: CancellationToken,
    wake: Arc<tokio::sync::Notify>,
) {
    let strategy = Arc::new(strategy);
    let mut changes = strategy.changes();
    let mut epoch = 0u64;
    let mut entries = HashMap::<S::Key, Entry<S::Key, S::Value, S::Error>>::new();
    let mut active: Option<Work<S::Key, S::Value, S::Error>> = None;
    let mut active_key = None;
    let mut refresh = tokio::time::interval(config.refresh_interval);
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    refresh.tick().await;
    let mut ready_at = Instant::now();
    loop {
        // Observe invalidations before serving cached data or publishing an evaluation.
        if changes.has_changed().unwrap_or(false) {
            changes.borrow_and_update();
            invalidate(&mut entries, &mut epoch, &mut ready_at, config.debounce);
        }
        let now = Instant::now();
        entries.retain(|key, entry| {
            entry.waiters.retain(|r| !r.reply.is_closed());
            !entry.waiters.is_empty()
                || active_key.as_ref() == Some(key)
                || (entry.mark.is_some() && now.duration_since(entry.used) < config.idle_ttl)
        });
        if active.is_none() && now >= ready_at {
            let candidate = entries
                .iter()
                .filter(|(_, e)| e.dirty)
                .min_by_key(|(_, e)| (e.waiters.is_empty(), e.used))
                .map(|(k, _)| k.clone());
            if let Some(key) = candidate {
                let backend = strategy.clone();
                let request = key.clone();
                let generation = epoch;
                let timeout = config.attempt_timeout;
                active_key = Some(key.clone());
                active = Some(Box::pin(async move {
                    let result = tokio::time::timeout(timeout, backend.evaluate(request))
                        .await
                        .map_err(|_| Error::TimedOut)
                        .and_then(|r| r.map_err(Error::Strategy));
                    (key, generation, result)
                }));
            }
        }
        tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            changed = changes.changed() => {
                if changed.is_err() { break; }
                invalidate(&mut entries, &mut epoch, &mut ready_at, config.debounce);
            },
            Some(mut request) = requests.recv() => {
                if request.reply.is_closed() { continue; }
                let normalized = match strategy.normalize(request.key.clone()) {
                    Ok(key) => key,
                    Err(error) => {
                        let _ = request.reply.send(Err(Error::Strategy(error)));
                        continue;
                    }
                };
                request.key = normalized.clone();
                if !entries.contains_key(&normalized) && entries.len() >= config.max_entries {
                    let oldest = entries.iter()
                        .filter(|(key, entry)| entry.waiters.is_empty() && active_key.as_ref() != Some(*key))
                        .min_by_key(|(_, entry)| entry.used)
                        .map(|(key, _)| key.clone());
                    if let Some(key) = oldest {
                        entries.remove(&key);
                    } else {
                        let _ = request.reply.send(Err(Error::Busy));
                        continue;
                    }
                }
                let entry = entries.entry(normalized).or_insert_with(|| Entry {
                    mark: None, dirty: true, used: Instant::now(), waiters: vec![],
                });
                entry.used = Instant::now();
                if !entry.dirty && let Some(mark) = &entry.mark {
                    let _ = request.reply.send(Ok(mark.clone()));
                } else {
                    entry.dirty = true;
                    entry.waiters.push(request);
                }
            },
            completed = async {
                match active.as_mut() {
                    Some(work) => work.await,
                    None => std::future::pending().await,
                }
            } => {
                active = None;
                active_key = None;
                let (key, generation, result) = completed;
                let stale = generation != epoch || changes.has_changed().unwrap_or(true);
                let Some(entry) = entries.get_mut(&key) else { continue; };
                // Do not publish an older evaluation as current after a source change.
                if stale { entry.dirty = true; continue; }
                entry.dirty = false;
                match result {
                    Ok(result) => {
                        let mark = Arc::new(Mark::evaluated(S::ID,result));
                        entry.mark = Some(mark.clone());
                        for waiter in entry.waiters.drain(..) {
                            let _ = waiter.reply.send(Ok(mark.clone()));
                        }
                    }
                    Err(error) => {
                        entry.mark = None;
                        for waiter in entry.waiters.drain(..) {
                            let _ = waiter.reply.send(Err(error.clone()));
                        }
                    }
                }
            },
            _ = wake.notified() => {},
            _ = refresh.tick() => invalidate(&mut entries, &mut epoch, &mut ready_at, Duration::ZERO),
            _ = tokio::time::sleep_until(ready_at), if active.is_none() && ready_at > Instant::now() => {},
        }
    }
}

fn invalidate<K, V, E>(
    entries: &mut HashMap<K, Entry<K, V, E>>,
    epoch: &mut u64,
    ready: &mut Instant,
    debounce: Duration,
) {
    *epoch = epoch.wrapping_add(1);
    let clean = entries.values().any(|entry| !entry.dirty);
    for entry in entries.values_mut() {
        entry.dirty = true;
    }
    // Coalesce a burst without postponing refresh indefinitely under continuous updates.
    if clean {
        *ready = Instant::now() + debounce;
    }
}
