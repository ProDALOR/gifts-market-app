//! Protocol-native, disposable read replica. No hub implementation dependencies.
mod book_index;
pub use book_index::OrderBookSnapshot;
mod limits;
mod query;
mod ranking;
mod read_options;
mod reader;
pub use limits::{READ_LIMITS, ReadLimits};
pub use read_options::{AttributeMatch, ReadOptions};
mod range;
#[cfg(test)]
mod range_tests;
mod read_key;
pub use range::{KeyRange, RangeRequest, ReadBatch, ReadEntry};
pub use types::Sort;
mod revisions;
pub use revisions::{DirectoryRevision, MetadataDependencies, MetadataRevision, ReadRevision};
mod replication;
mod runtime;
pub use replication::ReplicationWorker;
pub use runtime::ReplicaRuntime;
mod pending_history;
mod persistence;
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
#[derive(Clone)]
struct ReplicaView {
    revisions: revisions::Revisions,
    state: State,
    history: Snapshot,
    cursor: String,
    updated_at: Option<i64>,
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
#[derive(Default)]
enum PublicationState {
    #[default]
    Current,
    Uncertain,
    Prepared {
        view: Arc<ReplicaView>,
        data_changed: bool,
        directory_sync: Option<std::fs::File>,
    },
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
    publication: PublicationState,
    persistence: Option<persistence::Persistence>,
    #[cfg(test)]
    fail_publish_once: bool,
    #[cfg(test)]
    fail_directory_sync_once: bool,
    // Keep this last: release writer admission after database/publication handles.
    // The lock file itself is never renamed or deleted with a database generation.
    _directory_lock: std::fs::File,
}
impl ReplicaWriter {
    pub fn open(directory: &Path, source: &str) -> Result<Self, Status> {
        std::fs::create_dir_all(directory).map_err(storage::failure)?;
        let directory_lock = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("replica.lock"))
            .map_err(storage::failure)?;
        directory_lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => {
                Status::failed_precondition("replica data directory already has a writer")
            }
            std::fs::TryLockError::Error(error) => storage::failure(error),
        })?;
        let path = directory.join("replica.redb");
        let store = Store::open(&path, source)?;
        let building = directory.join("building.redb");
        if building.exists() {
            let started = std::time::Instant::now();
            tracing::info!("discarding interrupted replica snapshot build");
            std::fs::remove_file(building).map_err(storage::failure)?;
            tracing::info!(
                elapsed_ms = started.elapsed().as_millis(),
                "interrupted replica snapshot build discarded"
            );
        }
        let history = store.snapshot()?;
        let storage::Checkpoint { cursor, updated_at } = history.checkpoint()?;
        let published = if cursor.is_empty() {
            None
        } else {
            let state = history.state()?;
            let started = std::time::Instant::now();
            tracing::info!("rebuilding replica revision indexes");
            let revisions = revisions::Revisions::replaced(None, &state);
            tracing::info!(
                elapsed_ms = started.elapsed().as_millis(),
                "replica revision indexes restored"
            );
            Some(Arc::new(ReplicaView {
                revisions,
                state,
                history,
                cursor,
                updated_at,
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
            publication: PublicationState::Current,
            persistence: None,
            #[cfg(test)]
            fail_publish_once: false,
            #[cfg(test)]
            fail_directory_sync_once: false,
            _directory_lock: directory_lock,
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
    /// Retain active partitions for one collection/currency, without the full replica view.
    pub fn order_book_snapshot(
        &self,
        collection: &str,
        currency: &str,
        mode: w::OrderMode,
    ) -> Result<OrderBookSnapshot, Status> {
        self.check_cancelled()?;
        if collection.is_empty()
            || currency.is_empty()
            || !matches!(mode, w::OrderMode::Ask | w::OrderMode::Bid)
        {
            return Err(Status::invalid_argument("active book scope required"));
        }
        let scope = query::Scope::try_from(w::Scope {
            item_type_id: Some(collection.into()),
            currency_id: Some(currency.into()),
            modes: vec![mode as i32],
            ..Default::default()
        })?;
        Ok(OrderBookSnapshot::new(self.view()?.state.books(), scope))
    }
    fn pin(&self, view: Arc<ReplicaView>) -> Self {
        let (status, _) = watch::channel(self.status_for(&view));
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
    fn status_for(&self, view: &ReplicaView) -> SyncStatus {
        let mut status = self.status();
        status.ready = true;
        status.updated_at = view.updated_at;
        status
    }
    /// Monotonic publication sequence within this replica reader's lifetime.
    /// A pinned reader retains its sequence across updates and dataset replacement.
    pub fn generation(&self) -> Result<u64, Status> {
        Ok(self.view()?.revisions.sequence)
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
    fn resume_cursor(&mut self) -> Result<String, Status> {
        if let Err(error) = self.finish_persistence() {
            tracing::warn!(%error, "recovering replica from its durable checkpoint");
            self.publication = PublicationState::Uncertain;
        }
        if let PublicationState::Prepared { view, .. } = &self.publication {
            let cursor = view.cursor.clone();
            let availability = self.reader.status().state;
            self.publish_prepared(availability)?;
            return Ok(cursor);
        }
        let history = self.store.snapshot()?;
        let checkpoint = history.checkpoint()?;
        let previous = self.reader.view()?;
        if matches!(self.publication, PublicationState::Uncertain)
            || previous.cursor != checkpoint.cursor
        {
            let cursor = checkpoint.cursor.clone();
            let state = history.state()?;
            let availability = self.reader.status().state;
            self.install_state(history, state, checkpoint, availability)?;
            return Ok(cursor);
        }
        Ok(checkpoint.cursor)
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
        if !matches!(self.publication, PublicationState::Current) {
            self.resume_cursor()?;
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
        if !matches!(self.publication, PublicationState::Current) {
            self.resume_cursor()?;
        }
        self.finish_persistence()?;
        // Prepare everything fallible before changing the canonical file name.
        // Once rename succeeds, retain the new database and its directory barrier
        // even if syncing or publication fails; subsequent writes must recover it.
        let preparing = std::time::Instant::now();
        let history = snapshot.store.snapshot()?;
        let checkpoint = history.checkpoint()?;
        let view = self.replacement_view(history, snapshot.state, checkpoint);
        let directory = std::fs::File::open(
            self.path
                .parent()
                .ok_or_else(|| storage::failure("missing replica directory"))?,
        )
        .map_err(storage::failure)?;
        let prepare_time = preparing.elapsed();
        let renaming = std::time::Instant::now();
        std::fs::rename(self.path.with_file_name("building.redb"), &self.path)
            .map_err(storage::failure)?;
        let rename_time = renaming.elapsed();
        self.publication = PublicationState::Prepared {
            view,
            data_changed: true,
            directory_sync: Some(directory),
        };
        tracing::info!(
            prepare_ms = prepare_time.as_millis(),
            rename_ms = rename_time.as_millis(),
            "replica snapshot replacement prepared"
        );
        self.publish_prepared(SyncState::Live)
    }
}

impl ReplicaWriter {
    pub fn mark(&self, state: SyncState) {
        if self.reader.0.status.send_if_modified(|status| {
            if status.state == state {
                return false;
            }
            status.state = state;
            true
        }) {
            self.notify_changed();
        }
    }
    fn notify_changed(&self) {
        self.reader
            .0
            .changed
            .send_modify(|v| *v = v.wrapping_add(1));
    }
    #[cfg(test)]
    fn install(&mut self, store: Store) -> Result<(), Status> {
        let history = store.snapshot()?;
        let checkpoint = history.checkpoint()?;
        let state = history.state()?;
        self.install_state(history, state, checkpoint, SyncState::Live)
    }
    fn install_state(
        &mut self,
        history: Snapshot,
        state: State,
        checkpoint: storage::Checkpoint,
        availability: SyncState,
    ) -> Result<(), Status> {
        let view = self.replacement_view(history, state, checkpoint);
        self.publish(view, availability, true)
    }
    fn replacement_view(
        &self,
        history: Snapshot,
        state: State,
        checkpoint: storage::Checkpoint,
    ) -> Arc<ReplicaView> {
        self.reader.expire();
        let revisions = revisions::Revisions::replaced(self.reader.view().ok().as_deref(), &state);
        Arc::new(ReplicaView {
            revisions,
            state,
            history,
            cursor: checkpoint.cursor,
            updated_at: checkpoint.updated_at,
        })
    }
    pub fn apply(&mut self, batch: w::Batch) -> Result<(), Status> {
        self.apply_owned(vec![batch])
    }
    #[cfg(test)]
    fn apply_deliveries(&mut self, batches: &[w::Batch]) -> Result<(), Status> {
        self.apply_batches(std::borrow::Cow::Borrowed(batches))
    }
    fn apply_owned(&mut self, batches: Vec<w::Batch>) -> Result<(), Status> {
        self.apply_batches(std::borrow::Cow::Owned(batches))
    }
    fn apply_batches(&mut self, batches: std::borrow::Cow<'_, [w::Batch]>) -> Result<(), Status> {
        self.poll_persistence()?;
        let last = batches
            .last()
            .ok_or_else(|| Status::data_loss("missing replication delivery"))?;
        if batches.iter().any(|batch| batch.cursor.is_empty()) {
            return Err(Status::data_loss("empty replication checkpoint"));
        }
        if !matches!(self.publication, PublicationState::Current) {
            self.resume_cursor()?;
        }
        self.reader.expire();
        let previous = self.reader.view()?;
        if last.cursor == previous.cursor && batches.iter().all(|batch| batch.changes.is_empty()) {
            self.mark(SyncState::Live);
            return Ok(());
        }
        let changes = || batches.iter().flat_map(|batch| &batch.changes);
        let mut state = previous.state.clone();
        state.apply(changes())?;
        if self.persistence.is_none() {
            self.persistence = Some(persistence::Persistence::new(self.store.clone())?);
        }
        let sequence = self
            .persistence
            .as_ref()
            .expect("persistence")
            .next_sequence()?;
        let (history, history_changed) = previous.history.stage(sequence, changes())?;
        let revisions =
            previous
                .revisions
                .updated(&previous.state, &state, changes(), history_changed)?;
        let data_changed = revisions.sequence != previous.revisions.sequence;
        let view = Arc::new(ReplicaView {
            revisions,
            state,
            history,
            cursor: last.cursor.clone(),
            updated_at: Some(now()),
        });
        self.persistence
            .as_mut()
            .expect("persistence")
            .submit(batches.into_owned())?;
        self.publish(view, SyncState::Live, data_changed)
    }
    fn storage_completed(&mut self, completed: persistence::Completed) -> Result<(), Status> {
        let base = match &completed.snapshot {
            Ok(base) => base,
            Err(error) => {
                self.publication = PublicationState::Uncertain;
                self.mark(SyncState::Stale);
                tracing::error!(%error,"replica background persistence failed");
                return Err(error.clone());
            }
        };
        if let PublicationState::Prepared { view, .. } = &mut self.publication {
            *view = Arc::new(ReplicaView {
                history: view.history.persisted(base, completed.sequence)?,
                ..view.as_ref().clone()
            });
        } else if matches!(self.publication, PublicationState::Current) {
            let previous = self.reader.view()?;
            let view = Arc::new(ReplicaView {
                history: previous.history.persisted(base, completed.sequence)?,
                ..previous.as_ref().clone()
            });
            let old = self
                .reader
                .0
                .published
                .write()
                .map_err(storage::failure)?
                .replace(view);
            drop(old);
        }
        Ok(())
    }
    fn poll_persistence(&mut self) -> Result<(), Status> {
        while let Some(completed) = self
            .persistence
            .as_ref()
            .map(|persistence| persistence.completed())
            .transpose()
            .inspect_err(|_| {
                self.publication = PublicationState::Uncertain;
                self.mark(SyncState::Stale);
            })?
            .flatten()
        {
            self.storage_completed(completed)?;
        }
        Ok(())
    }
    fn finish_persistence(&mut self) -> Result<(), Status> {
        if let Some(persistence) = &mut self.persistence {
            persistence.finish();
        }
        let result = self.poll_persistence();
        self.persistence.take();
        result
    }
    fn publish(
        &mut self,
        view: Arc<ReplicaView>,
        availability: SyncState,
        data_changed: bool,
    ) -> Result<(), Status> {
        // Arc retention does not copy state. Retry this exact view and its scoped
        // revisions if publication fails, instead of rebuilding the dataset.
        self.publication = PublicationState::Prepared {
            view,
            data_changed,
            directory_sync: None,
        };
        self.publish_prepared(availability)
    }
    fn publish_prepared(&mut self, availability: SyncState) -> Result<(), Status> {
        let PublicationState::Prepared {
            view,
            data_changed,
            directory_sync,
        } = &mut self.publication
        else {
            return Err(Status::failed_precondition("no prepared publication"));
        };
        let replacement = directory_sync.is_some();
        let started = std::time::Instant::now();
        if let Some(directory) = directory_sync {
            #[cfg(test)]
            if std::mem::take(&mut self.fail_directory_sync_once) {
                return Err(Status::unavailable("test directory sync failure"));
            }
            directory.sync_all().map_err(storage::failure)?;
            *directory_sync = None;
        }
        let directory_time = started.elapsed();
        let switching = std::time::Instant::now();
        let view = view.clone();
        let data_changed = *data_changed;
        #[cfg(test)]
        if std::mem::take(&mut self.fail_publish_once) {
            return Err(Status::unavailable("test publication failure"));
        }
        let updated_at = view.updated_at;
        let old = {
            let mut published = self.reader.0.published.write().map_err(storage::failure)?;
            let old_store = std::mem::replace(&mut self.store, view.history._store.clone());
            (published.replace(view), old_store)
        };
        let status = SyncStatus {
            ready: true,
            state: availability,
            updated_at,
        };
        let mut availability_changed = false;
        self.reader.0.status.send_if_modified(|old| {
            availability_changed = old.ready != status.ready || old.state != status.state;
            if *old == status {
                return false;
            }
            *old = status;
            true
        });
        if data_changed || availability_changed {
            self.notify_changed();
        }
        self.publication = PublicationState::Current;
        let switch_time = switching.elapsed();
        // Old roots/database handles can be expensive to release after a full
        // replacement. Readers must not wait on that destruction under the lock.
        let retiring = std::time::Instant::now();
        drop(old);
        if replacement {
            tracing::info!(
                directory_sync_ms = directory_time.as_millis(),
                view_switch_ms = switch_time.as_millis(),
                retire_ms = retiring.elapsed().as_millis(),
                "replica snapshot publication timings"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod benchmarks;

mod blocking;
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

#[cfg(test)]
mod bootstrap_benchmark;

#[cfg(test)]
mod live_update_benchmark;

#[cfg(test)]
mod snapshot_recovery_tests;

impl Drop for ReplicaWriter {
    fn drop(&mut self) {
        if let Err(error) = self.finish_persistence() {
            tracing::warn!(%error, "replica persistence shutdown failed");
        }
    }
}
