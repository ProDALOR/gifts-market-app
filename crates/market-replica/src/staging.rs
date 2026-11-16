//! Unpublished snapshot construction with an on-disk change journal.
use crate::{
    state::State,
    storage::{self, Store},
};
use market_hub_protocol::wire as w;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tonic::Status;

pub struct SnapshotBuilder {
    store: Store,
    state: State,
    target: PathBuf,
    source: String,
    journal: SnapshotJournal,
    lease: SnapshotLease,
    owner: Arc<()>,
}
/// An opaque, finalized snapshot that only `ReplicaWriter` can publish.
pub struct PreparedSnapshot {
    pub(crate) store: Store,
    pub(crate) state: State,
    pub(crate) target: PathBuf,
    pub(crate) source: String,
    pub(crate) owner: Arc<()>,
    _lease: SnapshotLease,
}
/// A staging-only journal; all clones are closed before a snapshot is finalized.
#[derive(Clone)]
pub struct SnapshotJournal(Arc<Mutex<JournalState>>);
struct JournalState {
    store: Store,
    open: bool,
}
struct SnapshotLease {
    journal: SnapshotJournal,
    _token: Arc<()>,
}
impl Drop for SnapshotLease {
    fn drop(&mut self) {
        if let Ok(mut journal) = self.journal.0.lock() {
            journal.open = false;
        }
    }
}
impl SnapshotBuilder {
    pub(crate) fn open(
        target: &Path,
        source: &str,
        owner: Arc<()>,
        token: Arc<()>,
    ) -> Result<Self, Status> {
        let path = target.with_file_name("building.redb");
        if path.exists() {
            std::fs::remove_file(&path).map_err(storage::failure)?;
        }
        let store = Store::open(&path, source)?;
        let journal = SnapshotJournal(Arc::new(Mutex::new(JournalState {
            store: store.clone(),
            open: true,
        })));
        Ok(Self {
            lease: SnapshotLease {
                journal: journal.clone(),
                _token: token,
            },
            journal,
            owner,
            store,
            state: State::default(),
            target: target.to_owned(),
            source: source.into(),
        })
    }
    pub fn journal(&self) -> SnapshotJournal {
        self.journal.clone()
    }
    pub fn append(&mut self, changes: &[w::Change]) -> Result<(), Status> {
        let mut state = self.state.clone();
        state.apply(changes)?;
        self.store.apply(changes, None)?;
        self.state = state;
        Ok(())
    }
    pub fn replay(&mut self, position: u64) -> Result<Option<String>, Status> {
        if let Some(batch) = self.store.journal_at(position)? {
            self.append(&batch.changes)?;
            Ok(Some(batch.cursor))
        } else {
            Ok(None)
        }
    }
    pub fn finish(self, cursor: &str) -> Result<PreparedSnapshot, Status> {
        if cursor.is_empty() {
            return Err(Status::data_loss("missing snapshot checkpoint"));
        }
        // Serialize closing with admitted journal writes, including tracked storage jobs.
        self.journal.0.lock().map_err(storage::failure)?.open = false;
        self.store.finish(cursor)?;
        Ok(PreparedSnapshot {
            store: self.store,
            state: self.state,
            target: self.target,
            source: self.source,
            owner: self.owner,
            _lease: self.lease,
        })
    }
}
impl SnapshotJournal {
    pub fn push(&self, batch: &w::Batch) -> Result<(), Status> {
        let journal = self.0.lock().map_err(storage::failure)?;
        if !journal.open {
            return Err(Status::failed_precondition("snapshot journal is closed"));
        }
        journal.store.journal(batch)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ReplicaWriter, state,
        tests::{catalog, entity, order, read, upsert},
    };
    use market_hub_protocol::wire as w;

    #[test]
    fn snapshot_builds_are_exclusive_and_failed_appends_do_not_publish_partial_state() {
        let directory = tempfile::tempdir().unwrap();
        let mut writer = ReplicaWriter::open(directory.path(), "test").unwrap();
        let reader = writer.reader();
        let mut builder = writer.begin_snapshot().unwrap();
        let journal = builder.journal();
        assert!(writer.begin_snapshot().is_err());
        builder.append(&catalog()).unwrap();
        assert!(
            builder
                .append(&[
                    upsert(entity(w::entity::Entity::Order(order(
                        "rejected", "1", None
                    )))),
                    w::Change { change: None },
                ])
                .is_err()
        );
        assert!(!reader.status().ready);
        let batch = w::Batch {
            cursor: "c1".into(),
            changes: vec![upsert(entity(w::entity::Entity::Order(order(
                "accepted", "1", None,
            ))))],
        };
        journal.push(&batch).unwrap();
        assert_eq!(builder.replay(0).unwrap(), Some("c1".into()));
        assert_eq!(builder.replay(1).unwrap(), None);
        let snapshot = builder.finish("c1").unwrap();
        assert!(journal.push(&batch).is_err());
        assert!(
            writer.begin_snapshot().is_err(),
            "the prepared snapshot still owns the build lease"
        );
        writer.install_snapshot(snapshot).unwrap();
        let result = reader.read(read(1, false)).unwrap();
        assert_eq!(result.page.cursor, "c1");
        assert_eq!(result.page.entities.len(), 1);
        assert_eq!(
            state::order(&result.page.entities[0]).unwrap().id,
            "accepted"
        );
        let next = writer.begin_snapshot().unwrap();
        let abandoned_journal = next.journal();
        drop(next);
        assert!(abandoned_journal.push(&batch).is_err());
        assert!(writer.begin_snapshot().is_ok());
    }
    #[test]
    fn a_prepared_snapshot_cannot_replace_a_different_writer_even_in_the_same_directory() {
        let directory = tempfile::tempdir().unwrap();
        let mut old_writer = ReplicaWriter::open(directory.path(), "test").unwrap();
        let mut old_builder = old_writer.begin_snapshot().unwrap();
        old_builder.append(&catalog()).unwrap();
        let old_snapshot = old_builder.finish("old").unwrap();
        drop(old_writer);
        let mut writer = ReplicaWriter::open(directory.path(), "test").unwrap();
        let mut builder = writer.begin_snapshot().unwrap();
        builder.append(&catalog()).unwrap();
        let snapshot = builder.finish("current").unwrap();
        assert!(writer.install_snapshot(old_snapshot).is_err());
        assert_eq!(writer.cursor().unwrap(), "");
        writer.install_snapshot(snapshot).unwrap();
        assert_eq!(writer.cursor().unwrap(), "current");
        assert_eq!(
            writer.reader().read(read(1, false)).unwrap().page.cursor,
            "current"
        );
    }
}
