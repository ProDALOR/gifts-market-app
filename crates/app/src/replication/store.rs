use super::ReplicaView;
use market_view::protocol::wire as w;
use market_view::{Cursor, ItemTypeId, MarketEvent, MarketTimestamp, Record};
use prost::Message;
use std::{
    collections::VecDeque,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::watch;
use tonic::Status;

#[derive(Clone)]
pub struct ReplicaSnapshot {
    pub view: Option<Arc<ReplicaView>>,
    pub connected: bool,
    pub last_contact: Option<Instant>,
    pub last_contact_at: Option<MarketTimestamp>,
}
impl ReplicaSnapshot {
    pub fn age(&self) -> Option<Duration> {
        self.last_contact.map(|time| time.elapsed())
    }
    pub fn can_serve(&self, max_stale: Duration) -> bool {
        self.view.is_some() && self.age().is_some_and(|age| age <= max_stale)
    }
}
pub struct ReplayEvent {
    pub event: MarketEvent,
    pub previous_item_type: Option<ItemTypeId>,
}
pub struct ReplayBatch {
    pub previous: Cursor,
    pub cursor: Cursor,
    pub events: Vec<ReplayEvent>,
    pub catalog: Vec<market_view::CatalogEntry>,
    pub catalog_revision: String,
    bytes: usize,
}
struct Store {
    snapshot: ReplicaSnapshot,
    log: VecDeque<Arc<ReplayBatch>>,
    bytes: usize,
}
struct Inner {
    store: RwLock<Store>,
    changed: watch::Sender<u64>,
    max_batches: usize,
    max_bytes: usize,
}
#[derive(Clone)]
pub struct ReplicaHandle(Arc<Inner>);
impl Default for ReplicaHandle {
    fn default() -> Self {
        Self::new()
    }
}
impl ReplicaHandle {
    pub fn new() -> Self {
        Self::with_replay_limits(4096, 32 * 1024 * 1024)
    }
    pub fn with_replay_limits(max_batches: usize, max_bytes: usize) -> Self {
        assert!(max_batches > 0 && max_bytes > 0);
        let (changed, _) = watch::channel(0);
        Self(Arc::new(Inner {
            store: RwLock::new(Store {
                snapshot: ReplicaSnapshot {
                    view: None,
                    connected: false,
                    last_contact: None,
                    last_contact_at: None,
                },
                log: VecDeque::new(),
                bytes: 0,
            }),
            changed,
            max_batches,
            max_bytes,
        }))
    }
    pub fn snapshot(&self) -> ReplicaSnapshot {
        self.0
            .store
            .read()
            .expect("replica state poisoned")
            .snapshot
            .clone()
    }
    pub fn updates(&self) -> watch::Receiver<u64> {
        self.0.changed.subscribe()
    }
    pub fn next_batch(&self, after: &Cursor) -> Result<Option<Arc<ReplayBatch>>, Status> {
        let store = self
            .0
            .store
            .read()
            .map_err(|_| Status::unavailable("replica state unavailable"))?;
        let view = store
            .snapshot
            .view
            .as_ref()
            .ok_or_else(|| Status::unavailable("replica has no snapshot"))?;
        if after.generation != view.cursor.generation {
            return Err(Status::failed_precondition(
                "resync_required: generation_changed",
            ));
        }
        if after.revision > view.cursor.revision {
            return Err(Status::unavailable("replica_behind"));
        }
        let floor = store
            .log
            .front()
            .map_or(view.cursor.revision, |batch| batch.previous.revision);
        if after.revision < floor {
            return Err(Status::out_of_range("resync_required: replay_lost"));
        }
        Ok(store.log.get((after.revision - floor) as usize).cloned())
    }
    pub(super) fn activate(
        &self,
        view: Arc<ReplicaView>,
        connected: bool,
        events: Option<Vec<MarketEvent>>,
    ) {
        self.activate_catalog(view, connected, events, Vec::new());
    }
    pub(super) fn activate_catalog(
        &self,
        view: Arc<ReplicaView>,
        connected: bool,
        events: Option<Vec<MarketEvent>>,
        catalog: Vec<market_view::CatalogEntry>,
    ) {
        let mut store = self.0.store.write().expect("replica state poisoned");
        let previous = store.snapshot.view.clone();
        if let Some(previous) = previous.filter(|previous| previous.cursor != view.cursor) {
            match events.filter(|_| {
                previous.cursor.generation == view.cursor.generation
                    && previous.cursor.revision.checked_add(1) == Some(view.cursor.revision)
            }) {
                Some(events) => {
                    let events: Vec<_> = events
                        .into_iter()
                        .map(|event| {
                            let key = match &event {
                                MarketEvent::Upsert(record) => Some(record.key()),
                                MarketEvent::Delete(key) => Some(key.clone()),
                                _ => None,
                            };
                            let previous_item_type = key
                                .as_ref()
                                .and_then(|key| previous.state.get(key))
                                .and_then(Record::item_type_id)
                                .cloned();
                            ReplayEvent {
                                event,
                                previous_item_type,
                            }
                        })
                        .collect();
                    let bytes = catalog
                        .iter()
                        .cloned()
                        .map(w::CatalogEntry::from)
                        .map(|entry| entry.encoded_len() + 16)
                        .sum::<usize>()
                        + events
                            .iter()
                            .map(|event| {
                                w::MarketEvent::from(event.event.clone()).encoded_len()
                                    + event
                                        .previous_item_type
                                        .as_ref()
                                        .map_or(0, |id| id.as_str().len())
                                    + 64
                            })
                            .sum::<usize>()
                        + 128;
                    store.log.push_back(Arc::new(ReplayBatch {
                        previous: previous.cursor.clone(),
                        cursor: view.cursor.clone(),
                        events,
                        catalog,
                        catalog_revision: view.catalog.revision.clone(),
                        bytes,
                    }));
                    store.bytes = store.bytes.saturating_add(bytes);
                    while store.log.len() > self.0.max_batches || store.bytes > self.0.max_bytes {
                        if let Some(batch) = store.log.pop_front() {
                            store.bytes -= batch.bytes;
                        }
                    }
                }
                None => {
                    store.log.clear();
                    store.bytes = 0;
                }
            }
        }
        store.snapshot = ReplicaSnapshot {
            view: Some(view),
            connected,
            last_contact: Some(Instant::now()),
            last_contact_at: Some(MarketTimestamp::now()),
        };
        drop(store);
        self.0
            .changed
            .send_modify(|version| *version = version.wrapping_add(1));
    }
    pub(super) fn disconnected(&self) {
        self.0
            .store
            .write()
            .expect("replica state poisoned")
            .snapshot
            .connected = false;
        self.0
            .changed
            .send_modify(|version| *version = version.wrapping_add(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_query::{MarketState, PreparedChanges};
    use market_reader::registration::NodeCredentials;
    use market_view::protocol::catalog::CatalogSnapshot;
    use market_view::{
        Catalog, CatalogEntry, FeedDefinition, FeedState, FeedStatus, Mutation, VenueDefinition,
    };
    fn view() -> ReplicaView {
        let catalog = Arc::new(CatalogSnapshot::new(
            "generation".into(),
            Arc::new(
                Catalog::from_entries([
                    CatalogEntry::Venue(VenueDefinition {
                        id: "venue".parse().unwrap(),
                        name: "Venue".into(),
                    }),
                    CatalogEntry::Feed(FeedDefinition {
                        id: "feed".parse().unwrap(),
                        venue_id: "venue".parse().unwrap(),
                    }),
                ])
                .unwrap(),
            ),
        ));
        let mut state = MarketState::new(catalog.catalog.clone());
        state
            .apply_prepared(
                &PreparedChanges::new(
                    catalog.catalog.clone(),
                    vec![Mutation::Upsert(feed())],
                    |record| w::Record::from(record.clone()).encoded_len(),
                )
                .unwrap(),
            )
            .unwrap();
        ReplicaView {
            catalog,
            state,
            cursor: Cursor {
                generation: "generation".into(),
                revision: 1,
            },
        }
    }
    fn feed() -> Record {
        Record::Feed(FeedStatus {
            feed_id: "feed".parse().unwrap(),
            venue_id: "venue".parse().unwrap(),
            state: FeedState::Ready,
            last_sync_at: Some(MarketTimestamp::now()),
        })
    }
    fn config() -> super::super::ReplicaConfig {
        super::super::ReplicaConfig::new(
            "http://127.0.0.1:50052".into(),
            NodeCredentials {
                id: "reader".into(),
                role: w::NodeRole::Reader,
                token: "test-reader-token".into(),
            },
        )
    }
    #[test]
    fn replay_preserves_empty_revisions_and_rejects_gaps_and_generation_changes() {
        let handle = ReplicaHandle::with_replay_limits(2, 1024 * 1024);
        let mut view = view();
        let initial = view.cursor.clone();
        handle.activate(Arc::new(view.clone()), true, None);
        for revision in 2..=4 {
            let events = if revision == 3 {
                Vec::new()
            } else {
                vec![MarketEvent::Upsert(feed())]
            };
            view = view
                .apply(
                    Cursor {
                        generation: "generation".into(),
                        revision,
                    },
                    events.clone(),
                    &config(),
                )
                .unwrap();
            handle.activate(Arc::new(view.clone()), true, Some(events));
        }
        assert_eq!(
            handle.next_batch(&initial).err().unwrap().code(),
            tonic::Code::OutOfRange
        );
        let after = Cursor {
            generation: "generation".into(),
            revision: 2,
        };
        let empty = handle.next_batch(&after).unwrap().unwrap();
        assert_eq!(empty.previous, after);
        assert_eq!(empty.cursor.revision, 3);
        assert!(empty.events.is_empty());
        assert!(handle.next_batch(&view.cursor).unwrap().is_none());
        assert_eq!(
            handle
                .next_batch(&Cursor {
                    generation: "generation".into(),
                    revision: 5
                })
                .err()
                .unwrap()
                .code(),
            tonic::Code::Unavailable
        );
        // A new bootstrap can jump within a generation; it cannot claim to cover the gap.
        view.cursor.revision = 10;
        handle.activate(Arc::new(view.clone()), true, None);
        assert_eq!(
            handle.next_batch(&after).err().unwrap().code(),
            tonic::Code::OutOfRange
        );
        view.cursor.generation = "replacement".into();
        handle.activate(Arc::new(view), true, None);
        assert_eq!(
            handle.next_batch(&after).err().unwrap().code(),
            tonic::Code::FailedPrecondition
        );
    }
    #[test]
    fn byte_budget_evicts_oversized_batches_without_blocking_replication() {
        let handle = ReplicaHandle::with_replay_limits(100, 1);
        let view = view();
        let before = view.cursor.clone();
        handle.activate(Arc::new(view.clone()), true, None);
        let next = view
            .apply(
                Cursor {
                    generation: "generation".into(),
                    revision: 2,
                },
                Vec::new(),
                &config(),
            )
            .unwrap();
        handle.activate(Arc::new(next), true, Some(Vec::new()));
        assert_eq!(handle.snapshot().view.unwrap().cursor.revision, 2);
        assert_eq!(
            handle.next_batch(&before).err().unwrap().code(),
            tonic::Code::OutOfRange
        );
    }
    #[test]
    fn typed_scope_projects_moving_items_and_deleted_items_using_previous_type() {
        use crate::{query::Scope, streams::project};
        let item = market_view::Item {
            feed_id: "feed".parse().unwrap(),
            item_type_id: "new-type".parse().unwrap(),
            observed_at: MarketTimestamp::now(),
            data: market_view::ItemData {
                item_id: "item".parse().unwrap(),
                attributes: Default::default(),
            },
        };
        let record = Record::Item(item);
        let key = record.key();
        let batch = ReplayBatch {
            previous: Cursor {
                generation: "g".into(),
                revision: 1,
            },
            cursor: Cursor {
                generation: "g".into(),
                revision: 2,
            },
            events: vec![
                ReplayEvent {
                    event: MarketEvent::Upsert(record.clone()),
                    previous_item_type: Some("old-type".parse().unwrap()),
                },
                ReplayEvent {
                    event: MarketEvent::Delete(key.clone()),
                    previous_item_type: Some("new-type".parse().unwrap()),
                },
            ],
            catalog: Vec::new(),
            catalog_revision: String::new(),
            bytes: 1,
        };
        let old = Scope {
            item_type_id: Some("old-type".parse().unwrap()),
            ..Default::default()
        };
        let new = Scope {
            item_type_id: Some("new-type".parse().unwrap()),
            ..Default::default()
        };
        assert_eq!(
            project(&old, &batch).collect::<Vec<_>>(),
            vec![MarketEvent::Delete(key.clone())]
        );
        assert_eq!(
            project(&new, &batch).collect::<Vec<_>>(),
            vec![MarketEvent::Upsert(record), MarketEvent::Delete(key)]
        );
        assert!(
            project(
                &Scope {
                    feed_id: Some("other-feed".parse().unwrap()),
                    ..old
                },
                &batch
            )
            .next()
            .is_none()
        );
    }
}
