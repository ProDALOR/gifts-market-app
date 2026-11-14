use crate::{
    api::*,
    codec,
    error::{ApiError, Result},
};
use hub_protocol::wire as w;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, watch};
#[derive(Clone)]
pub struct Gateway {
    pub replica: market_replica::Replica,
    queries: Arc<Semaphore>,
    streams: Arc<Semaphore>,
    live: Arc<Mutex<HashMap<String, Weak<Live>>>>,
}
pub struct Live {
    pub sender: watch::Sender<Update>,
    done: AtomicBool,
}
#[derive(Clone)]
pub enum Update {
    Connecting,
    Page(Arc<Page<Order>>),
    Error(ApiError),
}
fn capacity() -> ApiError {
    tonic::Status::resource_exhausted("gateway capacity exhausted").into()
}
fn limit(value: u32) -> Result<u32> {
    if !(1..=200).contains(&value) {
        Err(ApiError::invalid("limit must be 1..200"))
    } else {
        Ok(value)
    }
}
impl Gateway {
    pub fn new(replica: market_replica::Replica, queries: usize, streams: usize) -> Self {
        Self {
            replica,
            queries: Arc::new(Semaphore::new(queries)),
            streams: Arc::new(Semaphore::new(streams)),
            live: Arc::default(),
        }
    }
    pub async fn ready(&self) -> Result<()> {
        if self.replica.status().ready {
            Ok(())
        } else {
            Err(tonic::Status::unavailable("Market replica is loading").into())
        }
    }
    pub fn sync(&self) -> SyncInfo {
        self.replica.status().into()
    }
    pub async fn books(&self, q: BooksQuery) -> Result<BooksPage> {
        let _permit = self
            .queries
            .clone()
            .try_acquire_owned()
            .map_err(|_| capacity())?;
        let limit = if q.limit == 0 { 200 } else { limit(q.limit)? };
        let replica = self.replica.clone();
        let page = tokio::task::spawn_blocking(move || {
            replica.books(q.after.map(Into::into), limit as usize)
        })
        .await
        .map_err(|_| ApiError::from(tonic::Status::unavailable("Read worker stopped")))??;
        Ok(BooksPage {
            data: page.books.into_iter().map(Into::into).collect(),
            next_after: page.next_after.map(Into::into),
        })
    }
    async fn snapshot(&self, mut query: w::ReadRequest) -> Result<market_replica::ReadResult> {
        let _permit = self
            .queries
            .clone()
            .try_acquire_owned()
            .map_err(|_| capacity())?;
        query.mode = w::ReadMode::Snapshot as i32;
        let replica = self.replica.clone();
        tokio::task::spawn_blocking(move || replica.read(query))
            .await
            .map_err(|_| ApiError::from(tonic::Status::unavailable("Read worker stopped")))?
            .map_err(Into::into)
    }
    pub async fn catalog(&self, q: CatalogQuery) -> Result<Page<CatalogEntry>> {
        if q.ids.len() > 500 {
            return Err(ApiError::invalid("at most 500 catalog IDs"));
        }
        let kind = q.kind.wire() as i32;
        codec::catalog(
            self.snapshot(w::ReadRequest {
                selection: Some(w::Selection {
                    kinds: vec![kind],
                    keys: q
                        .ids
                        .into_iter()
                        .map(|id| w::Key {
                            kind,
                            id,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }),
                limit: limit(q.limit)?,
                page_token: q.page_token,
                ..Default::default()
            })
            .await?,
        )
    }
    pub fn orders_request(mut q: OrdersQuery, mode: w::ReadMode) -> Result<w::ReadRequest> {
        if q.scope.item_type_id.as_ref().is_none_or(String::is_empty)
            || q.scope.currency_id.as_ref().is_none_or(String::is_empty)
        {
            return Err(ApiError::invalid("select an item type and quote currency"));
        }
        if q.scope.modes.is_empty() {
            q.scope.modes = vec![Mode::Ask];
        }
        if q.scope.modes.contains(&Mode::Executed) {
            return Err(ApiError::invalid(
                "use the executions query for executed orders",
            ));
        }
        Ok(w::ReadRequest {
            selection: Some(w::Selection {
                source: Some(w::selection::Source::Scope(q.scope.try_into()?)),
                ..Default::default()
            }),
            limit: limit(q.limit)?,
            page_token: q.page_token,
            mode: mode as i32,
            sort: w::ReadSort::from(q.sort) as i32,
        })
    }
    pub async fn orders(&self, q: OrdersQuery) -> Result<Page<Order>> {
        codec::orders(
            self.snapshot(Self::orders_request(q, w::ReadMode::Snapshot)?)
                .await?,
        )
    }
    pub async fn executions(&self, mut q: ExecutionsQuery) -> Result<Page<Order>> {
        q.scope.modes = vec![Mode::Executed];
        codec::orders(
            self.snapshot(w::ReadRequest {
                selection: Some(w::Selection {
                    source: Some(w::selection::Source::Scope(q.scope.try_into()?)),
                    executed_at: q.period.map(|p| w::Period {
                        from_ms: p.from.map(|v| v.as_millisecond()),
                        until_ms: p.until.map(|v| v.as_millisecond()),
                    }),
                    ..Default::default()
                }),
                limit: limit(q.limit)?,
                page_token: q.page_token,
                ..Default::default()
            })
            .await?,
        )
    }
    /// Identical queries share one upstream LIVE stream. Only the latest bounded page is retained.
    pub fn subscribe(&self, q: OrdersQuery) -> Result<(Arc<Live>, watch::Receiver<Update>)> {
        let request = Self::orders_request(q.clone(), w::ReadMode::Live)?;
        let key = serde_json::to_string(&q).map_err(|_| ApiError::invalid("invalid query"))?;
        let mut live = self.live.lock().map_err(|_| capacity())?;
        live.retain(|_, v| v.strong_count() > 0);
        if let Some(feed) = live
            .get(&key)
            .and_then(Weak::upgrade)
            .filter(|f| !f.done.load(Ordering::Acquire))
        {
            let rx = feed.sender.subscribe();
            return Ok((feed, rx));
        }
        let permit = self
            .streams
            .clone()
            .try_acquire_owned()
            .map_err(|_| capacity())?;
        let (sender, rx) = watch::channel(Update::Connecting);
        let feed = Arc::new(Live {
            sender,
            done: AtomicBool::new(false),
        });
        live.insert(key, Arc::downgrade(&feed));
        let state = self.clone();
        let weak = Arc::downgrade(&feed);
        let sender = feed.sender.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let mut changed = state.replica.changes();
            let work = async {
                let mut previous: Option<Arc<Page<Order>>> = None;
                loop {
                    changed.borrow_and_update();
                    let worker = state
                        .queries
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| capacity())?;
                    let replica = state.replica.clone();
                    let query = request.clone();
                    let result = tokio::task::spawn_blocking(move || replica.read(query))
                        .await
                        .map_err(|_| {
                            ApiError::from(tonic::Status::unavailable("Read worker stopped"))
                        })??;
                    drop(worker);
                    let page = Arc::new(codec::orders(result)?);
                    if previous.as_ref().is_none_or(|old| {
                        old.data != page.data
                            || old.next_page_token != page.next_page_token
                            || old.sync != page.sync
                    }) {
                        previous = Some(page.clone());
                        sender.send_replace(Update::Page(page));
                    }
                    tokio::select! {
                        _ = sender.closed() => return Ok::<(),ApiError>(()),
                        result = changed.changed() => { result.map_err(|_| ApiError::from(tonic::Status::unavailable("Replica stopped")))?; },
                        _ = tokio::time::sleep(Duration::from_secs(10)) => { previous = None; }
                    }
                    tokio::select! { _ = sender.closed() => return Ok(()), _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
                }
            };
            let result = work.await;
            if let Some(feed) = weak.upgrade() {
                feed.done.store(true, Ordering::Release);
            }
            if let Err(error) = result {
                sender.send_replace(Update::Error(error));
            }
        });
        Ok((feed, rx))
    }
}
