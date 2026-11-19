//! Protocol-native, disposable read replica. No hub implementation dependencies.
mod book_index;
mod limits;
mod query;
mod ranking;
mod reader;
pub use limits::{READ_LIMITS, ReadLimits};
mod read_key;
mod slice;
#[cfg(test)]
mod slice_tests;
pub use slice::{KeyRange, Navigation, ReadBatch, ReadEntry, SliceRequest};
mod revisions;
pub use revisions::{DirectoryRevision, MetadataDependencies, MetadataRevision, ReadRevision};
mod replication;
mod runtime;
pub use replication::ReplicationWorker;
pub use runtime::ReplicaRuntime;
mod staging;
mod state;
mod storage;
#[cfg(test)]
mod test_support;
pub use staging::{PreparedSnapshot, SnapshotBuilder, SnapshotJournal};
mod types;
use market_hub_protocol::wire as w;
use serde::{Deserialize, Serialize};
use state::State;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, Weak},
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
#[derive(Clone)]
pub struct ReadResult {
    pub page: w::Page,
    pub sync: SyncStatus,
}
struct ReplicaView {
    revisions: revisions::Revisions,
    state: State,
    history: Snapshot,
    cursor: String,
}
struct RetainedView {
    view: Arc<ReplicaView>,
    until: Instant,
}
struct ReadState {
    published: RwLock<Option<Arc<ReplicaView>>>,
    status: watch::Sender<SyncStatus>,
    changed: watch::Sender<u64>,
    views: Mutex<BTreeMap<String, RetainedView>>,
}
/// Read-only access to immutable views published by synchronization.
#[derive(Clone)]
pub struct ReplicaReader(Arc<ReadState>, tokio_util::sync::CancellationToken);
/// Exclusive, non-cloneable mutation engine, owned by synchronization.
/// Cloning a `ReplicaReader` never grants this capability.
pub struct ReplicaWriter {
    path: PathBuf,
    source: String,
    staging: Weak<()>,
    owner: Arc<()>,
    store: Store,
    reader: ReplicaReader,
}
impl ReplicaWriter {
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
            let state = store.state()?;
            Some(Arc::new(ReplicaView {
                revisions: revisions::Revisions::replaced(None, &state),
                state,
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
        let replica = ReplicaReader(
            Arc::new(ReadState {
                published: RwLock::new(published),
                status,
                changed,
                views: Mutex::default(),
            }),
            tokio_util::sync::CancellationToken::new(),
        );
        Ok(Self {
            path,
            source: source.into(),
            staging: Weak::new(),
            owner: Arc::new(()),
            store,
            reader: replica,
        })
    }
}
impl ReplicaReader {
    /// Pin one published view for a short, compound read without blocking synchronization.
    /// Attach request cancellation without changing the live or pinned view.
    pub fn with_cancellation(&self, cancelled: tokio_util::sync::CancellationToken) -> Self {
        Self(self.0.clone(), cancelled)
    }
    pub(crate) fn cancellation_token(&self) -> &tokio_util::sync::CancellationToken {
        &self.1
    }
    pub fn check_cancelled(&self) -> Result<(), Status> {
        if self.1.is_cancelled() {
            Err(Status::cancelled("Market read cancelled"))
        } else {
            Ok(())
        }
    }
    pub fn snapshot(&self) -> Result<Self, Status> {
        Ok(self.pin(self.view()?))
    }
    fn pin(&self, view: Arc<ReplicaView>) -> Self {
        let (status, _) = watch::channel(self.status());
        let (changed, _) = watch::channel(0);
        Self(
            Arc::new(ReadState {
                published: RwLock::new(Some(view)),
                status,
                changed,
                views: Mutex::default(),
            }),
            self.1.clone(),
        )
    }
    pub fn status(&self) -> SyncStatus {
        self.0.status.borrow().clone()
    }
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.0.changed.subscribe()
    }
    fn view(&self) -> Result<Arc<ReplicaView>, Status> {
        self.check_cancelled()?;
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
    pub fn books(&self, after: Option<w::BookKey>, limit: usize) -> Result<w::BooksPage, Status> {
        if limit == 0 || limit > 500 {
            return Err(Status::invalid_argument("invalid page size"));
        }
        Ok(self.view()?.state.books().page(after, limit))
    }
}

impl ReplicaWriter {
    pub fn reader(&self) -> ReplicaReader {
        self.reader.clone()
    }
    pub fn cursor(&self) -> Result<String, Status> {
        self.store.cursor()
    }
    pub fn expire_views(&self) {
        self.reader.expire();
    }
    pub fn begin_snapshot(&mut self) -> Result<SnapshotBuilder, Status> {
        if self.staging.upgrade().is_some() {
            return Err(Status::failed_precondition(
                "a replica snapshot is already being built",
            ));
        }
        let lease = Arc::new(());
        let snapshot =
            SnapshotBuilder::open(&self.path, &self.source, self.owner.clone(), lease.clone())?;
        self.staging = Arc::downgrade(&lease);
        Ok(snapshot)
    }
    pub fn install_snapshot(&mut self, snapshot: PreparedSnapshot) -> Result<(), Status> {
        if !Arc::ptr_eq(&snapshot.owner, &self.owner)
            || snapshot.target != self.path
            || snapshot.source != self.source
        {
            return Err(Status::failed_precondition(
                "snapshot belongs to another replica or source",
            ));
        }
        std::fs::rename(self.path.with_file_name("building.redb"), &self.path)
            .map_err(storage::failure)?;
        std::fs::File::open(
            self.path
                .parent()
                .ok_or_else(|| storage::failure("missing replica directory"))?,
        )
        .map_err(storage::failure)?
        .sync_all()
        .map_err(storage::failure)?;
        self.install_state(snapshot.store, snapshot.state)
    }
}

impl ReplicaWriter {
    pub fn mark(&self, state: SyncState) {
        let mut status = self.reader.status();
        status.state = state;
        self.reader.0.status.send_replace(status);
        self.reader
            .0
            .changed
            .send_modify(|v| *v = v.wrapping_add(1));
    }
    #[cfg(test)]
    fn install(&mut self, store: Store) -> Result<(), Status> {
        let state = store.state()?;
        self.install_state(store, state)
    }
    fn install_state(&mut self, store: Store, state: State) -> Result<(), Status> {
        self.reader.expire();
        let revisions = revisions::Revisions::replaced(self.reader.view().ok().as_deref(), &state);
        let view = Arc::new(ReplicaView {
            revisions,
            state,
            history: store.snapshot()?,
            cursor: store.cursor()?,
        });
        self.store = store.clone();
        *self.reader.0.published.write().map_err(storage::failure)? = Some(view);
        self.reader.0.status.send_replace(SyncStatus {
            ready: true,
            state: SyncState::Live,
            updated_at: store.updated_at()?,
        });
        self.reader
            .0
            .changed
            .send_modify(|v| *v = v.wrapping_add(1));
        Ok(())
    }
    pub fn apply(&mut self, batch: w::Batch) -> Result<(), Status> {
        if batch.cursor.is_empty() {
            return Err(Status::data_loss("empty replication checkpoint"));
        }
        self.reader.expire();
        let previous = self.reader.view()?;
        let mut state = previous.state.clone();
        state.apply(&batch.changes)?;
        let revisions = previous
            .revisions
            .updated(&previous.state, &state, &batch.changes)?;
        let store = &self.store;
        store.apply(&batch.changes, Some(&batch.cursor))?;
        let view = Arc::new(ReplicaView {
            revisions,
            state,
            history: store.snapshot()?,
            cursor: batch.cursor,
        });
        *self.reader.0.published.write().map_err(storage::failure)? = Some(view);
        self.reader.0.status.send_replace(SyncStatus {
            ready: true,
            state: SyncState::Live,
            updated_at: store.updated_at()?,
        });
        self.reader
            .0
            .changed
            .send_modify(|v| *v = v.wrapping_add(1));
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod benchmarks;

mod execution;
pub use execution::{ReadBudget, ReadExecutor, ReadPermit, ReadRuntime};
pub mod catalog_read;

#[cfg(test)]
mod query_benchmark;

pub use read_key::{ReadPosition, SnapshotId};

impl ReplicaReader {
    pub fn status_updates(&self) -> watch::Receiver<SyncStatus> {
        self.0.status.subscribe()
    }
}
