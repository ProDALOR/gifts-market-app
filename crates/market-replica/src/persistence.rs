//! Ordered bounded writes; live publication never waits for redb or fsync.
use crate::storage::{Snapshot, Store};
use market_hub_protocol::wire as w;
use prost::Message;
use std::sync::{Arc, mpsc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonic::Status;
const MAX_JOBS: usize = 256;
const MAX_BYTES: usize = 16 * 1024 * 1024;
pub(super) struct Completed {
    pub sequence: u64,
    pub snapshot: Result<Snapshot, Status>,
    _permits: Vec<OwnedSemaphorePermit>,
}
struct Write {
    sequence: u64,
    batches: Vec<w::Batch>,
    permits: Vec<OwnedSemaphorePermit>,
}
pub(super) struct Persistence {
    sender: Option<mpsc::Sender<Write>>,
    worker: Option<std::thread::JoinHandle<()>>,
    completed: mpsc::Receiver<Completed>,
    slots: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    sequence: u64,
    panicked: bool,
}
impl Persistence {
    pub fn new(store: Store) -> Result<Self, Status> {
        let (sender, receiver) = mpsc::channel::<Write>();
        let (complete, completed) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("replica-persistence".into())
            .spawn(move || {
                while let Ok(first) = receiver.recv() {
                    let mut sequence = first.sequence;
                    let mut writes = vec![first.batches];
                    let mut permits = first.permits;
                    while let Ok(next) = receiver.try_recv() {
                        sequence = next.sequence;
                        writes.push(next.batches);
                        permits.extend(next.permits);
                    }
                    let last = writes
                        .last()
                        .and_then(|batches| batches.last())
                        .expect("nonempty write");
                    let snapshot = store
                        .apply(
                            writes.iter().flatten().flat_map(|batch| &batch.changes),
                            Some(&last.cursor),
                        )
                        .map_err(|error| {
                            tracing::error!(
                                commit_attempted = error.commit_attempted,
                                "replica background commit failed"
                            );
                            error.status
                        })
                        .and_then(|_| store.snapshot());
                    let failed = snapshot.is_err();
                    if complete
                        .send(Completed {
                            sequence,
                            snapshot,
                            _permits: permits,
                        })
                        .is_err()
                        || failed
                    {
                        break;
                    }
                }
            })
            .map_err(|error| {
                Status::unavailable(format!("cannot start replica persistence: {error}"))
            })?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            completed,
            slots: Arc::new(Semaphore::new(MAX_JOBS)),
            bytes: Arc::new(Semaphore::new(MAX_BYTES)),
            sequence: 0,
            panicked: false,
        })
    }
    pub fn next_sequence(&self) -> Result<u64, Status> {
        self.sequence
            .checked_add(1)
            .ok_or_else(|| Status::resource_exhausted("persistence sequence exhausted"))
    }
    pub fn submit(&mut self, batches: Vec<w::Batch>) -> Result<u64, Status> {
        let bytes = batches
            .iter()
            .map(|batch| batch.encoded_len() + 512 + batch.changes.len() * 256)
            .sum::<usize>();
        let permits = vec![
            self.slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| Status::resource_exhausted("replica persistence queue is full"))?,
            self.bytes
                .clone()
                .try_acquire_many_owned(u32::try_from(bytes.max(1)).map_err(|_| {
                    Status::resource_exhausted("replica persistence byte limit reached")
                })?)
                .map_err(|_| {
                    Status::resource_exhausted("replica persistence byte limit reached")
                })?,
        ];
        let sequence = self.next_sequence()?;
        self.sender
            .as_ref()
            .ok_or_else(|| Status::unavailable("replica persistence stopped"))?
            .send(Write {
                sequence,
                batches,
                permits,
            })
            .map_err(|_| Status::unavailable("replica persistence stopped"))?;
        self.sequence = sequence;
        Ok(sequence)
    }
    pub fn completed(&self) -> Result<Option<Completed>, Status> {
        if self.panicked {
            return Err(Status::unavailable("replica persistence worker panicked"));
        }
        match self.completed.try_recv() {
            Ok(completed) => Ok(Some(completed)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) if self.worker.is_none() => Ok(None),
            Err(_) => Err(Status::unavailable("replica persistence worker stopped")),
        }
    }
    pub fn finish(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            self.panicked = worker.join().is_err();
        }
    }
}
impl Drop for Persistence {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocked_writes_are_bounded_and_shutdown_drains_the_ordered_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(
            &directory.path().join("replica.redb"),
            "bounded-persistence",
        )
        .unwrap();
        let blocked = store.hold_write_for_test();
        let mut persistence = Persistence::new(store.clone()).unwrap();
        for sequence in 0..MAX_JOBS {
            persistence
                .submit(vec![w::Batch {
                    cursor: format!("c{sequence}"),
                    changes: vec![],
                }])
                .unwrap();
        }
        assert_eq!(
            persistence
                .submit(vec![w::Batch {
                    cursor: "overflow".into(),
                    changes: vec![]
                }])
                .unwrap_err()
                .code(),
            tonic::Code::ResourceExhausted
        );
        assert!(store.cursor().unwrap().is_empty());
        drop(blocked);
        persistence.finish();
        while let Some(completed) = persistence.completed().unwrap() {
            completed.snapshot.as_ref().unwrap();
        }
        assert_eq!(persistence.slots.available_permits(), MAX_JOBS);
        assert_eq!(persistence.bytes.available_permits(), MAX_BYTES);
        assert_eq!(store.cursor().unwrap(), format!("c{}", MAX_JOBS - 1));
    }
}
