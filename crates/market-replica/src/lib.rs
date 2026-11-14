//! Protocol-native, disposable read replica. No hub implementation dependencies.
mod query;
mod read;
mod state;
mod storage;
mod sync;
use hub_protocol::wire as w;
use serde::{Deserialize, Serialize};
use state::State;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use storage::{Snapshot, Store};
use tokio::sync::watch;
use tonic::Status;

pub fn now() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Starting,
    Syncing,
    Live,
    Stale,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStatus {
    pub ready: bool,
    pub state: SyncState,
    pub updated_at: Option<i64>,
}
pub struct ReadResult {
    pub page: w::Page,
    pub sync: SyncStatus,
}
struct View {
    state: State,
    history: Snapshot,
    cursor: String,
}
struct Retained {
    view: Arc<View>,
    until: Instant,
}
struct Inner {
    path: PathBuf,
    source: String,
    store: Mutex<Store>,
    published: RwLock<Option<Arc<View>>>,
    status: watch::Sender<SyncStatus>,
    changed: watch::Sender<u64>,
    views: Mutex<BTreeMap<String, Retained>>,
    writer: tokio::sync::Mutex<()>,
}
#[derive(Clone)]
pub struct Replica(Arc<Inner>);
impl Replica {
    pub fn open(directory: &Path, source: &str) -> Result<Self, Status> {
        let path = directory.join("replica.redb");
        let store = Store::open(&path, source)?;
        let building = directory.join("building.redb");
        if building.exists() {
            std::fs::remove_file(building).map_err(storage::failure)?;
        }
        let cursor = store.cursor()?;
        let updated_at = store.updated_at()?;
        let published = if cursor.is_empty() {
            None
        } else {
            Some(Arc::new(View {
                state: store.state()?,
                history: store.snapshot()?,
                cursor,
            }))
        };
        let ready = published.is_some();
        let (status, _) = watch::channel(SyncStatus {
            ready,
            state: if ready {
                SyncState::Stale
            } else {
                SyncState::Starting
            },
            updated_at,
        });
        let (changed, _) = watch::channel(0);
        Ok(Self(Arc::new(Inner {
            path,
            source: source.into(),
            store: Mutex::new(store),
            published: RwLock::new(published),
            status,
            changed,
            views: Mutex::default(),
            writer: tokio::sync::Mutex::new(()),
        })))
    }
    pub fn status(&self) -> SyncStatus {
        self.0.status.borrow().clone()
    }
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.0.changed.subscribe()
    }
    fn view(&self) -> Result<Arc<View>, Status> {
        self.0
            .published
            .read()
            .map_err(storage::failure)?
            .clone()
            .ok_or_else(|| Status::unavailable("market replica is loading"))
    }
    fn expire(&self) {
        if let Ok(mut views) = self.0.views.lock() {
            views.retain(|_, v| v.until > Instant::now());
        }
    }
    fn mark(&self, state: SyncState) {
        let mut status = self.status();
        status.state = state;
        self.0.status.send_replace(status);
        self.0.changed.send_modify(|v| *v = v.wrapping_add(1));
    }
    #[cfg(test)]
    fn install(&self, store: Store) -> Result<(), Status> {
        let state = store.state()?;
        self.install_state(store, state)
    }
    fn install_state(&self, store: Store, state: State) -> Result<(), Status> {
        let view = Arc::new(View {
            state,
            history: store.snapshot()?,
            cursor: store.cursor()?,
        });
        *self.0.store.lock().map_err(storage::failure)? = store.clone();
        *self.0.published.write().map_err(storage::failure)? = Some(view);
        self.0.status.send_replace(SyncStatus {
            ready: true,
            state: SyncState::Live,
            updated_at: store.updated_at()?,
        });
        self.0.changed.send_modify(|v| *v = v.wrapping_add(1));
        Ok(())
    }
    fn apply(&self, batch: w::Batch) -> Result<(), Status> {
        if batch.cursor.is_empty() {
            return Err(Status::data_loss("empty replication checkpoint"));
        }
        let mut state = self.view()?.state.clone();
        state.apply(&batch.changes)?;
        let store = self.0.store.lock().map_err(storage::failure)?.clone();
        store.apply(&batch.changes, Some(&batch.cursor))?;
        let view = Arc::new(View {
            state,
            history: store.snapshot()?,
            cursor: batch.cursor,
        });
        *self.0.published.write().map_err(storage::failure)? = Some(view);
        self.0.status.send_replace(SyncStatus {
            ready: true,
            state: SyncState::Live,
            updated_at: store.updated_at()?,
        });
        self.0.changed.send_modify(|v| *v = v.wrapping_add(1));
        Ok(())
    }
    pub fn books(&self, after: Option<w::BookKey>, limit: usize) -> Result<w::BooksPage, Status> {
        if limit == 0 || limit > 500 {
            return Err(Status::invalid_argument("invalid page size"));
        }
        use std::ops::Bound::{Excluded, Unbounded};
        let view = self.view()?;
        let books = view.state.books();
        let after = after.as_ref().map(state::book);
        let mut rows = books.range((after.map_or(Unbounded, Excluded), Unbounded));
        let data: Vec<_> = rows
            .by_ref()
            .take(limit)
            .map(|((venue, item, currency), _)| w::BookKey {
                venue_id: venue.clone(),
                item_type_id: item.clone(),
                currency_id: currency.clone(),
            })
            .collect();
        Ok(w::BooksPage {
            next_after: if rows.next().is_some() {
                data.last().cloned()
            } else {
                None
            },
            books: data,
        })
    }
    pub async fn run(
        &self,
        channel: tonic::transport::Channel,
        authorization: tonic::metadata::MetadataValue<tonic::metadata::Ascii>,
        stop: tokio_util::sync::CancellationToken,
    ) {
        let _writer = self.0.writer.lock().await;
        let task = sync::run(self.clone(), channel, authorization, stop.clone());
        tokio::pin!(task);
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {_=&mut task=>break,_=interval.tick()=>self.expire()}
        }
    }
}

#[cfg(test)]
mod tests;
