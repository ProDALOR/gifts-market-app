//! Unpublished snapshot construction with an on-disk change journal.
use crate::{
    state::State,
    storage::{self, Store},
};
use market_hub_protocol::wire as w;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tonic::Status;

pub struct SnapshotBuilder {
    store: Store,
    state: Option<State>,
    timings: BuildTimings,
    target: PathBuf,
    source: String,
    journal: SnapshotJournal,
    lease: SnapshotLease,
    owner: Arc<()>,
}
/// Accumulated successful staging/replay work, excluding worker queue time.
#[derive(Clone, Copy, Default)]
pub(crate) struct BuildTimings {
    pub transactions: u64,
    pub state: Duration,
    pub db_wait: Duration,
    pub db_write: Duration,
    pub db_commit: Duration,
}
pub(crate) struct Replayed {
    pub next_position: u64,
    pub cursor: String,
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
pub struct SnapshotJournal(Arc<JournalState>);
struct JournalState {
    store: Store,
    open: AtomicBool,
    writes: Mutex<Vec<u8>>,
    lease: Weak<()>,
}
struct SnapshotLease {
    journal: SnapshotJournal,
    _token: Arc<()>,
}
impl Drop for SnapshotLease {
    fn drop(&mut self) {
        // Closing on cancellation must not wait for a database writer. An
        // admitted push owns its own lease until its transaction has finished.
        self.journal.0.open.store(false, Ordering::Release);
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
        let journal = SnapshotJournal(Arc::new(JournalState {
            store: store.clone(),
            open: AtomicBool::new(true),
            writes: Mutex::new(Vec::new()),
            lease: Arc::downgrade(&token),
        }));
        Ok(Self {
            lease: SnapshotLease {
                journal: journal.clone(),
                _token: token,
            },
            journal,
            owner,
            store,
            state: Some(State::default()),
            timings: BuildTimings::default(),
            target: target.to_owned(),
            source: source.into(),
        })
    }
    pub fn journal(&self) -> SnapshotJournal {
        self.journal.clone()
    }
    pub fn append(&mut self, changes: &[w::Change]) -> Result<(), Status> {
        self.append_changes(changes)
    }
    pub(crate) fn timings(&self) -> BuildTimings {
        self.timings
    }
    fn append_changes<'a>(
        &mut self,
        changes: impl IntoIterator<Item = &'a w::Change> + Clone,
    ) -> Result<(), Status> {
        // No published view can share these roots. Any failure leaves None:
        // production abandons the build, so rebuilding it just to discard it
        // wastes a complete database scan and duplicates its RAM allocations.
        let mut state = self.state.take().ok_or_else(failed_builder)?;
        let started = Instant::now();
        state.apply(changes.clone())?;
        let state_time = started.elapsed();
        let applied = self.store.apply(changes, None)?;
        self.state = Some(state);
        self.timings.transactions += 1;
        self.timings.state += state_time;
        self.timings.db_wait += applied.timings.wait;
        self.timings.db_write += applied.timings.write;
        self.timings.db_commit += applied.timings.commit;
        Ok(())
    }
    pub fn replay(&mut self, position: u64) -> Result<Option<String>, Status> {
        Ok(self
            .replay_up_to(position, 1, usize::MAX)?
            .map(|replayed| replayed.cursor))
    }
    pub(crate) fn replay_chunk(&mut self, position: u64) -> Result<Option<Replayed>, Status> {
        self.replay_up_to(
            position,
            crate::limits::MAX_APPLY_BATCHES,
            crate::limits::MAX_APPLY_BYTES,
        )
    }
    fn replay_up_to(
        &mut self,
        position: u64,
        deliveries: usize,
        bytes: usize,
    ) -> Result<Option<Replayed>, Status> {
        if self.state.is_none() {
            return Err(failed_builder());
        }
        let mut batches = self.store.journal_range(position, deliveries, bytes)?;
        if batches.is_empty() {
            return Ok(None);
        }
        if batches.iter().any(|batch| batch.cursor.is_empty()) {
            return Err(Status::data_loss("empty snapshot journal checkpoint"));
        }
        let next_position = position + batches.len() as u64;
        if batches.iter().any(|batch| !batch.changes.is_empty()) {
            // Borrow decoded changes directly for both passes. Flattening into
            // a second Vec copied every large Change wrapper during catch-up.
            self.append_changes(batches.iter().flat_map(|batch| &batch.changes))?;
        }
        Ok(Some(Replayed {
            next_position,
            cursor: batches.pop().unwrap().cursor,
        }))
    }
    pub fn finish(self, cursor: &str) -> Result<PreparedSnapshot, Status> {
        if cursor.is_empty() {
            return Err(Status::data_loss("missing snapshot checkpoint"));
        }
        let state = self.state.ok_or_else(failed_builder)?;
        // Serialize closing with admitted journal writes, including tracked storage jobs.
        self.journal.close()?;
        self.store.finish(cursor)?;
        Ok(PreparedSnapshot {
            store: self.store,
            state,
            target: self.target,
            source: self.source,
            owner: self.owner,
            _lease: self.lease,
        })
    }
}
fn failed_builder() -> Status {
    Status::failed_precondition("snapshot builder must be discarded after a failed batch")
}
impl SnapshotJournal {
    pub fn push(&self, batch: &w::Batch) -> Result<(), Status> {
        let mut buffer = self.0.writes.lock().map_err(storage::failure)?;
        if !self.0.open.load(Ordering::Acquire) {
            return Err(Status::failed_precondition("snapshot journal is closed"));
        }
        let _lease = self
            .0
            .lease
            .upgrade()
            .ok_or_else(|| Status::failed_precondition("snapshot journal is closed"))?;
        self.0.store.journal(batch, &mut buffer)
    }
    fn close(&self) -> Result<(), Status> {
        self.0.open.store(false, Ordering::Release);
        // Finalization must wait for admitted pushes before flushing/removing
        // their journal. Waiting pushes see closed and cannot write afterwards.
        drop(self.0.writes.lock().map_err(storage::failure)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ReplicaWriter, state,
        tests::{catalog, entity, order, read, upsert},
    };
    use market_hub_protocol::wire as w;

    #[tokio::test]
    async fn abandoning_a_builder_does_not_wait_for_an_admitted_journal_write() {
        let directory = tempfile::tempdir().unwrap();
        let mut writer = ReplicaWriter::open(directory.path(), "test").unwrap();
        let builder = writer.begin_snapshot().unwrap();
        let transaction = builder.store.hold_write_for_test();
        let journal = builder.journal();
        let writing = journal.clone();
        let job = tokio::task::spawn_blocking(move || {
            writing.push(&w::Batch {
                cursor: "queued".into(),
                changes: vec![],
            })
        });
        let admitted = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while journal.0.lease.strong_count() < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        if !admitted {
            drop(transaction);
            let _ = job.await;
            panic!("journal write was not admitted");
        }
        let (dropped, done) = tokio::sync::oneshot::channel();
        let dropping = std::thread::spawn(move || {
            drop(builder);
            let _ = dropped.send(());
        });
        let prompt = tokio::time::timeout(std::time::Duration::from_millis(100), done)
            .await
            .is_ok();
        let exclusive = writer.begin_snapshot().is_err();
        // Release before asserting, so an old blocking destructor cannot hang a failed test.
        drop(transaction);
        let result = job.await.unwrap();
        dropping.join().unwrap();
        assert!(
            prompt,
            "builder drop must not wait on journal/database locks"
        );
        assert!(
            exclusive,
            "admitted writes retain the exclusive build lease"
        );
        result.unwrap();
        assert!(
            journal
                .push(&w::Batch {
                    cursor: "closed".into(),
                    changes: vec![]
                })
                .is_err()
        );
        assert!(writer.begin_snapshot().is_ok());
    }

    #[tokio::test]
    async fn prefetch_progresses_during_journal_contention_and_waits_for_accepted_writes() {
        use futures_util::StreamExt;
        use tokio_util::task::TaskTracker;
        let directory = tempfile::tempdir().unwrap();
        let mut writer = ReplicaWriter::open(directory.path(), "test").unwrap();
        let mut builder = writer.begin_snapshot().unwrap();
        let transaction = builder.store.hold_write_for_test();
        let journal = builder.journal();
        let mut stream = futures_util::stream::iter([Ok(w::Batch {
            cursor: "queued".into(),
            changes: vec![],
        })])
        .chain(futures_util::stream::pending());
        let (advance, ready) = tokio::sync::oneshot::channel();
        let (progress, progressed) = tokio::sync::oneshot::channel();
        let (finish, finished) = tokio::sync::oneshot::channel();
        let work = async {
            ready.await.unwrap();
            let _ = progress.send(());
            finished.await.unwrap();
            Ok(42)
        };
        let mut queued = 0;
        let mut elapsed = std::time::Duration::ZERO;
        let jobs = TaskTracker::new();
        let mut collecting = Box::pin(crate::replication::collecting(
            work,
            &mut stream,
            &journal,
            &mut queued,
            &mut elapsed,
            &jobs,
        ));
        let admitted = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::select! {
                _ = async { while journal.0.lease.strong_count() < 2 { tokio::task::yield_now().await; } } => true,
                _ = &mut collecting => false,
            }
        }).await.unwrap_or(false);
        let _ = advance.send(());
        let made_progress = tokio::time::timeout(std::time::Duration::from_millis(100), async {
            tokio::select! { _ = progressed => true, _ = &mut collecting => false }
        })
        .await
        .unwrap_or(false);
        let _ = finish.send(());
        let waited = tokio::time::timeout(std::time::Duration::from_millis(25), &mut collecting)
            .await
            .is_err();
        drop(transaction);
        let result = collecting.await.unwrap();
        jobs.close();
        jobs.wait().await;
        assert!(
            admitted && made_progress,
            "prefetch must keep polling during writer contention"
        );
        assert!(
            waited,
            "completed work cannot bypass an accepted journal write"
        );
        assert_eq!(result, 42);
        assert_eq!(queued, 1);
        assert_eq!(builder.replay(0).unwrap(), Some("queued".into()));
    }

    #[test]
    fn uncertain_staging_commits_cannot_be_finished_or_reused() {
        for committed in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut writer = ReplicaWriter::open(directory.path(), "test").unwrap();
            let mut initial = writer.begin_snapshot().unwrap();
            initial.append(&catalog()).unwrap();
            writer
                .install_snapshot(initial.finish("old").unwrap())
                .unwrap();
            let mut builder = writer.begin_snapshot().unwrap();
            builder.append(&catalog()).unwrap();
            builder.store.fail_commit_once(committed);
            assert!(
                builder
                    .append(&[upsert(entity(w::entity::Entity::Order(order(
                        "new", "1", None
                    ))))])
                    .is_err()
            );
            assert!(builder.append(&[]).is_err());
            assert!(builder.replay(0).is_err());
            assert!(builder.finish("new").is_err());
            assert_eq!(writer.cursor().unwrap(), "old");
            drop(writer);
            let mut reopened = ReplicaWriter::open(directory.path(), "test").unwrap();
            assert_eq!(reopened.cursor().unwrap(), "old");
            assert!(reopened.begin_snapshot().is_ok());
        }
    }

    #[test]
    fn failed_staging_storage_validation_aborts_without_rebuilding_ram() {
        let directory = tempfile::tempdir().unwrap();
        let mut writer = ReplicaWriter::open(directory.path(), "test").unwrap();
        let mut builder = writer.begin_snapshot().unwrap();
        builder.append(&catalog()).unwrap();
        builder
            .append(&[upsert(entity(w::entity::Entity::Order(order(
                "sale",
                "1",
                Some(1),
            ))))])
            .unwrap();
        // RAM accepts execution removal, while storage rejects changing immutable history.
        let rejected = upsert(entity(w::entity::Entity::Order(order("new", "2", None))));
        let conflict = upsert(entity(w::entity::Entity::Order(order(
            "sale",
            "3",
            Some(1),
        ))));
        assert!(builder.append(&[rejected, conflict]).is_err());
        assert!(builder.state.is_none());
        assert!(
            builder
                .store
                .state()
                .unwrap()
                .entity(
                    &state::entity_key(&entity(w::entity::Entity::Order(order("new", "2", None))))
                        .unwrap()
                )
                .is_none()
        );
        assert!(builder.finish("failed").is_err());
        let mut builder = writer.begin_snapshot().unwrap();
        builder.append(&catalog()).unwrap();
        builder
            .append(&[upsert(entity(w::entity::Entity::Order(order(
                "accepted", "4", None,
            ))))])
            .unwrap();
        writer
            .install_snapshot(builder.finish("ok").unwrap())
            .unwrap();
        let result = writer.reader().read(read(1, false)).unwrap();
        assert_eq!(result.page.entities.len(), 1);
        assert_eq!(
            state::order(&result.page.entities[0]).unwrap().id,
            "accepted"
        );
        drop(writer);
        let writer = ReplicaWriter::open(directory.path(), "test").unwrap();
        let result = writer.reader().read(read(1, false)).unwrap();
        assert_eq!(result.page.entities.len(), 1);
        assert_eq!(
            state::order(&result.page.entities[0]).unwrap().id,
            "accepted"
        );
    }

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
        assert!(builder.state.is_none());
        assert!(builder.finish("failed").is_err());
        let batch = w::Batch {
            cursor: "c1".into(),
            changes: vec![upsert(entity(w::entity::Entity::Order(order(
                "accepted", "1", None,
            ))))],
        };
        assert!(journal.push(&batch).is_err());
        let mut builder = writer.begin_snapshot().unwrap();
        builder.append(&catalog()).unwrap();
        let journal = builder.journal();
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
    #[test]
    fn grouped_replay_is_ordered_bounded_atomic_and_durable() {
        let directory = tempfile::tempdir().unwrap();
        {
            let mut writer = ReplicaWriter::open(directory.path(), "grouped-replay").unwrap();
            let reader = writer.reader();
            let mut builder = writer.begin_snapshot().unwrap();
            builder.append(&catalog()).unwrap();
            builder
                .append(&[upsert(entity(w::entity::Entity::Order(order(
                    "listing", "999", None,
                ))))])
                .unwrap();
            let journal = builder.journal();
            for index in 0..130 {
                let mut changes = if index % 3 == 0 {
                    vec![]
                } else {
                    vec![upsert(entity(w::entity::Entity::Order(order(
                        "listing",
                        &index.to_string(),
                        None,
                    ))))]
                };
                if index == 65 {
                    changes.push(upsert(entity(w::entity::Entity::Order(order(
                        "sale",
                        "2",
                        Some(1),
                    )))));
                }
                journal
                    .push(&w::Batch {
                        cursor: format!("c{index}"),
                        changes,
                    })
                    .unwrap();
            }
            let mut position = 0;
            let mut cursor = String::new();
            for expected in [64, 128, 130] {
                let replayed = builder.replay_chunk(position).unwrap().unwrap();
                assert_eq!(replayed.next_position, expected);
                assert_eq!(replayed.cursor, format!("c{}", expected - 1));
                position = replayed.next_position;
                cursor = replayed.cursor;
                assert!(
                    !reader.status().ready,
                    "Staged replay must remain unpublished"
                );
            }
            assert!(builder.replay_chunk(position).unwrap().is_none());
            let prepared = builder.finish(&cursor).unwrap();
            assert!(
                prepared
                    .store
                    .journal_range(0, 1, usize::MAX)
                    .unwrap()
                    .is_empty()
            );
            writer.install_snapshot(prepared).unwrap();
            assert_eq!(reader.cursor().unwrap(), "c129");
            assert_eq!(
                state::order(&reader.read(read(1, false)).unwrap().page.entities[0])
                    .unwrap()
                    .price,
                "128"
            );
            assert_eq!(reader.read(read(0, true)).unwrap().page.entities.len(), 1);
        }
        let restored = ReplicaWriter::open(directory.path(), "grouped-replay").unwrap();
        assert_eq!(restored.cursor().unwrap(), "c129");
        assert_eq!(
            state::order(
                &restored
                    .reader()
                    .read(read(1, false))
                    .unwrap()
                    .page
                    .entities[0]
            )
            .unwrap()
            .price,
            "128"
        );
        assert_eq!(
            restored
                .reader()
                .read(read(0, true))
                .unwrap()
                .page
                .entities
                .len(),
            1
        );
    }
    #[test]
    fn replay_respects_byte_budget_and_rejects_a_whole_invalid_chunk() {
        let directory = tempfile::tempdir().unwrap();
        let mut writer = ReplicaWriter::open(directory.path(), "bounded-replay").unwrap();
        let mut builder = writer.begin_snapshot().unwrap();
        builder.append(&catalog()).unwrap();
        let journal = builder.journal();
        for index in 0..2 {
            let mut row = order(&format!("large-{index}"), "1", None);
            row.meta.insert("payload".into(), "x".repeat(600 * 1024));
            journal
                .push(&w::Batch {
                    cursor: format!("c{index}"),
                    changes: vec![upsert(entity(w::entity::Entity::Order(row)))],
                })
                .unwrap();
        }
        assert_eq!(builder.replay_chunk(0).unwrap().unwrap().next_position, 1);
        assert_eq!(builder.replay_chunk(1).unwrap().unwrap().next_position, 2);
        let valid = entity(w::entity::Entity::Order(order("rejected", "1", None)));
        let key = state::entity_key(&valid).unwrap();
        journal
            .push(&w::Batch {
                cursor: "valid-prefix".into(),
                changes: vec![upsert(valid)],
            })
            .unwrap();
        journal
            .push(&w::Batch {
                cursor: "invalid-tail".into(),
                changes: vec![w::Change { change: None }],
            })
            .unwrap();
        assert!(builder.replay_chunk(2).is_err());
        assert!(builder.state.is_none());
        assert!(builder.store.state().unwrap().entity(&key).is_none());
        assert!(builder.finish("failed").is_err());
        assert!(!writer.reader().status().ready);
    }
}
