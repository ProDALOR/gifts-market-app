use super::*;
use w::entity::Entity as E;
pub(super) fn entity(value: E) -> w::Entity {
    w::Entity {
        entity: Some(value),
    }
}
pub(super) fn upsert(value: w::Entity) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Upsert(value)),
    }
}
pub(super) fn catalog() -> Vec<w::Change> {
    vec![
        upsert(entity(E::Venue(w::Venue {
            id: "venue".into(),
            name: "Venue".into(),
        }))),
        upsert(entity(E::Currency(w::Currency {
            id: "GRAM".into(),
            name: "Gram".into(),
            decimals: 9,
        }))),
        upsert(entity(E::Currency(w::Currency {
            id: "XTR".into(),
            name: "Stars".into(),
            decimals: 9,
        }))),
        upsert(entity(E::ItemType(w::ItemType {
            id: "gift".into(),
            name: "Gift".into(),
            quantity_step: "1".into(),
            attributes: [("stickers".into(), w::AttributeRule::default())].into(),
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "stickers".into(),
            name: "Stickers".into(),
            value_type: 7,
            element: Some("sticker".into()),
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "sticker".into(),
            name: "Sticker".into(),
            value_type: 8,
            fields: [
                ("name".into(), w::AttributeRule::default()),
                ("wear".into(), w::AttributeRule::default()),
            ]
            .into(),
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "name".into(),
            name: "Name".into(),
            value_type: 5,
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "wear".into(),
            name: "Wear".into(),
            value_type: 3,
            ..Default::default()
        }))),
    ]
}
pub(super) fn order(id: &str, price: &str, time: Option<i64>) -> w::Order {
    w::Order {
        id: id.into(),
        book: Some(w::BookKey {
            venue_id: "venue".into(),
            item_type_id: "gift".into(),
            currency_id: "GRAM".into(),
        }),
        mode: if time.is_some() { 3 } else { 1 },
        price: price.into(),
        quantity: "1".into(),
        observed_at_ms: 1000,
        executed_at_ms: time,
        ..Default::default()
    }
}
pub(super) fn read(mode: i32, history: bool) -> w::ReadRequest {
    w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(w::Scope {
                item_type_id: Some("gift".into()),
                currency_id: Some("GRAM".into()),
                modes: vec![if history { 3 } else { 1 }],
                ..Default::default()
            })),
            ..Default::default()
        }),
        mode,
        sort: if history { 0 } else { 1 },
        limit: 1,
        ..Default::default()
    }
}
// Pure state/storage tests use a local writer. Production writes are only exposed
// through the actor's private replication capability (covered in actor tests).
struct Fixture {
    reader: ReplicaReader,
    writer: std::sync::Mutex<ReplicaWriter>,
}
impl Fixture {
    fn open(directory: &std::path::Path, source: &str) -> Result<Self, Status> {
        let writer = ReplicaWriter::open(directory, source)?;
        Ok(Self {
            reader: writer.reader.clone(),
            writer: std::sync::Mutex::new(writer),
        })
    }
    fn apply(&self, batch: w::Batch) -> Result<(), Status> {
        self.writer.lock().unwrap().apply(batch)
    }
    fn install(&self, store: Store) -> Result<(), Status> {
        self.writer.lock().unwrap().install(store)
    }
}
impl std::ops::Deref for Fixture {
    type Target = ReplicaReader;
    fn deref(&self) -> &ReplicaReader {
        &self.reader
    }
}
fn setup() -> (tempfile::TempDir, Fixture) {
    let dir = tempfile::tempdir().unwrap();
    let replica = Fixture::open(dir.path(), "test-source").unwrap();
    let store = replica.writer.lock().unwrap().store.clone();
    store.apply(&catalog(), Some("c0")).unwrap();
    replica.install(store).unwrap();
    (dir, replica)
}
fn apply(fixture: &Fixture, changes: Vec<w::Change>, cursor: &str) {
    fixture
        .apply(w::Batch {
            changes,
            cursor: cursor.into(),
        })
        .unwrap();
}
fn ids(page: &w::Page) -> Vec<String> {
    page.entities
        .iter()
        .map(|e| state::order(e).unwrap().id.clone())
        .collect()
}
#[test]
fn local_price_pages_are_exact_currency_isolated_and_share_live_snapshot_boundaries() {
    let (_dir, replica) = setup();
    let mut stars = order("stars", "0.000001", None);
    stars.book.as_mut().unwrap().currency_id = "XTR".into();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order(
                "z",
                "10000000000000000.000000001",
                None,
            )))),
            upsert(entity(E::Order(order(
                "a",
                "10000000000000000.000000002",
                None,
            )))),
            upsert(entity(E::Order(stars))),
        ],
        "c1",
    );
    let first = replica.read(read(1, false)).unwrap();
    assert_eq!(ids(&first.page), ["z"]);
    let mut query = read(0, false);
    query.page_token = first.page.next_page_token;
    let second = replica.read(query).unwrap();
    assert_eq!(ids(&second.page), ["a"]);
    let mut descending = read(1, false);
    descending.sort = 2;
    assert_eq!(ids(&replica.read(descending).unwrap().page), ["a"]);
    let mut query = read(1, false);
    query.limit = 10;
    let scope = match query.selection.as_mut().unwrap().source.as_mut().unwrap() {
        w::selection::Source::Scope(s) => s,
        _ => unreachable!(),
    };
    scope.price = Some(w::PriceRange {
        min: Some("10000000000000000.000000002".into()),
        max: None,
    });
    assert_eq!(ids(&replica.read(query).unwrap().page), ["a"]);
    let first = replica.read(read(0, false)).unwrap();
    let mut changed = read(1, false);
    changed.sort = 2;
    changed.page_token = first.page.next_page_token;
    assert_eq!(
        replica.read(changed).err().unwrap().code(),
        tonic::Code::InvalidArgument
    );
}
#[test]
fn terminal_transitions_late_backfills_and_snapshots_remain_consistent() {
    let (dir, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("b", "2", Some(20))))),
            upsert(entity(E::Order(order("c", "3", Some(30))))),
        ],
        "c1",
    );
    let initial = replica.read(read(0, true)).unwrap();
    assert_eq!(ids(&initial.page), ["b"]);
    apply(
        &replica,
        vec![upsert(entity(E::Order(order("a", "1", Some(10)))))],
        "c2",
    );
    assert!(
        replica
            .read(read(1, false))
            .unwrap()
            .page
            .entities
            .is_empty()
    );
    let mut continuation = read(0, true);
    continuation.page_token = initial.page.next_page_token;
    assert_eq!(ids(&replica.read(continuation).unwrap().page), ["c"]);
    let mut history = read(0, true);
    history.limit = 20;
    assert_eq!(
        ids(&replica.read(history.clone()).unwrap().page),
        ["a", "b", "c"]
    );
    let period = history.selection.as_mut().unwrap();
    period.executed_at = Some(w::Period {
        from_ms: Some(10),
        until_ms: Some(20),
    });
    assert_eq!(ids(&replica.read(history).unwrap().page), ["a"]);
    // An execution retry advances the durable cursor without adding another payload.
    let mut retry = order("a", "1", Some(10));
    retry.observed_at_ms = 2000;
    apply(&replica, vec![upsert(entity(E::Order(retry)))], "c3");
    drop(replica);
    let reopened = Fixture::open(dir.path(), "test-source").unwrap();
    assert!(reopened.status().ready);
    assert_eq!(reopened.status().state, SyncState::Stale);
    let mut history = read(0, true);
    history.limit = 20;
    let result = reopened.read(history).unwrap();
    assert_eq!(ids(&result.page), ["a", "b", "c"]);
    assert_eq!(result.page.cursor, "c3");
}
#[test]
fn rejected_batches_persist_neither_current_changes_nor_checkpoint() {
    let (dir, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("done", "2", Some(10))))),
        ],
        "good",
    );
    let error = replica
        .apply(w::Batch {
            changes: vec![
                upsert(entity(E::Order(order("a", "9", None)))),
                upsert(entity(E::Order(order("done", "99", Some(10))))),
            ],
            cursor: "bad".into(),
        })
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::DataLoss);
    assert_eq!(
        state::order(&replica.read(read(1, false)).unwrap().page.entities[0])
            .unwrap()
            .price,
        "1"
    );
    drop(replica);
    let restored = Fixture::open(dir.path(), "test-source").unwrap();
    assert_eq!(restored.view().unwrap().cursor, "good");
    assert_eq!(
        state::order(&restored.read(read(1, false)).unwrap().page.entities[0])
            .unwrap()
            .price,
        "1"
    );
}

#[test]
fn repeated_heartbeats_preserve_views_revisions_and_notification_quietness() {
    let (_dir, replica) = setup();
    let before = replica.view().unwrap();
    let updated_at = replica.writer.lock().unwrap().store.updated_at().unwrap();
    let changes = replica.changes();
    let statuses = replica.status_updates();
    for _ in 0..10 {
        apply(&replica, vec![], "c0");
        replica.writer.lock().unwrap().mark(SyncState::Live);
    }
    assert!(Arc::ptr_eq(&before, &replica.view().unwrap()));
    assert_eq!(
        replica.writer.lock().unwrap().store.updated_at().unwrap(),
        updated_at
    );
    assert!(!changes.has_changed().unwrap());
    assert!(!statuses.has_changed().unwrap());

    replica.writer.lock().unwrap().mark(SyncState::Stale);
    let mut changes = replica.changes();
    apply(&replica, vec![], "c0");
    assert_eq!(replica.status().state, SyncState::Live);
    assert!(
        changes.has_changed().unwrap(),
        "reconnect ACK must restore availability"
    );
    changes.borrow_and_update();
    replica.writer.lock().unwrap().mark(SyncState::Live);
    assert!(!changes.has_changed().unwrap());
}

#[test]
fn checkpoint_only_and_identical_updates_do_not_invalidate_data_but_remain_durable() {
    let (dir, replica) = setup();
    let listing = entity(E::Order(order("listing", "1", None)));
    let execution = entity(E::Order(order("sale", "2", Some(10))));
    apply(
        &replica,
        vec![upsert(listing.clone()), upsert(execution)],
        "facts",
    );
    let view = replica.view().unwrap();
    let reference = view
        .state
        .entity(&state::entity_key(&listing).unwrap())
        .unwrap()
        .clone();
    let all = replica.read_revision(&w::ReadRequest::default()).unwrap();
    let history = replica.read_revision(&read(0, true)).unwrap();
    let changes = replica.changes();
    let mut retry = order("sale", "2", Some(10));
    retry.observed_at_ms += 1;
    apply(
        &replica,
        vec![upsert(listing), upsert(entity(E::Order(retry)))],
        "identical",
    );
    apply(&replica, vec![], "checkpoint");
    assert_eq!(
        all,
        replica.read_revision(&w::ReadRequest::default()).unwrap()
    );
    assert_eq!(history, replica.read_revision(&read(0, true)).unwrap());
    assert!(!changes.has_changed().unwrap());
    assert!(Arc::ptr_eq(
        &reference,
        replica
            .view()
            .unwrap()
            .state
            .entity(&state::entity_key(&reference).unwrap())
            .unwrap()
    ));
    drop(view);
    drop(replica);
    let restored = Fixture::open(dir.path(), "test-source").unwrap();
    assert_eq!(restored.cursor().unwrap(), "checkpoint");
    assert_eq!(
        ids(&restored.read(read(1, false)).unwrap().page),
        ["listing"]
    );
    assert_eq!(ids(&restored.read(read(0, true)).unwrap().page), ["sale"]);
}

#[test]
fn grouped_deliveries_publish_once_and_reject_invalid_later_deliveries_atomically() {
    let (dir, replica) = setup();
    let mut changes = replica.changes();
    let deliveries = [
        w::Batch {
            cursor: "one".into(),
            changes: vec![upsert(entity(E::Order(order("a", "1", None))))],
        },
        w::Batch {
            cursor: "two".into(),
            changes: vec![upsert(entity(E::Order(order("a", "2", None))))],
        },
    ];
    let sequence = *changes.borrow_and_update();
    replica
        .writer
        .lock()
        .unwrap()
        .apply_deliveries(&deliveries)
        .unwrap();
    assert_eq!(*changes.borrow_and_update(), sequence + 1);
    assert_eq!(replica.cursor().unwrap(), "two");
    assert_eq!(
        state::order(&replica.read(read(1, false)).unwrap().page.entities[0])
            .unwrap()
            .price,
        "2"
    );
    let reference = replica.view().unwrap();
    let bad = [
        w::Batch {
            cursor: "rejected".into(),
            changes: vec![upsert(entity(E::Order(order("a", "9", None))))],
        },
        w::Batch {
            cursor: "bad".into(),
            changes: vec![w::Change { change: None }],
        },
    ];
    assert!(
        replica
            .writer
            .lock()
            .unwrap()
            .apply_deliveries(&bad)
            .is_err()
    );
    assert!(Arc::ptr_eq(&reference, &replica.view().unwrap()));
    assert!(!changes.has_changed().unwrap());
    drop(reference);
    drop(replica);
    let restored = Fixture::open(dir.path(), "test-source").unwrap();
    assert_eq!(restored.cursor().unwrap(), "two");
    assert_eq!(
        state::order(&restored.read(read(1, false)).unwrap().page.entities[0])
            .unwrap()
            .price,
        "2"
    );
}

#[test]
fn resume_repairs_committed_but_unpublished_current_and_history_without_marking_live() {
    let (_dir, replica) = setup();
    let mut writer = replica.writer.lock().unwrap();
    writer.mark(SyncState::Stale);
    let all = replica.read_revision(&w::ReadRequest::default()).unwrap();
    let history = replica.read_revision(&read(0, true)).unwrap();
    // Simulate a successful durable commit followed by a failed read-view creation.
    writer
        .store
        .apply(
            &[
                upsert(entity(E::Order(order("listing", "3", None)))),
                upsert(entity(E::Order(order("sale", "2", Some(10))))),
            ],
            Some("committed"),
        )
        .unwrap();
    assert_eq!(replica.cursor().unwrap(), "c0");
    assert_eq!(writer.resume_cursor().unwrap(), "committed");
    assert_eq!(replica.cursor().unwrap(), "committed");
    assert_eq!(replica.status().state, SyncState::Stale);
    assert_ne!(
        all,
        replica.read_revision(&w::ReadRequest::default()).unwrap()
    );
    assert_ne!(history, replica.read_revision(&read(0, true)).unwrap());
    assert_eq!(
        ids(&replica.read(read(1, false)).unwrap().page),
        ["listing"]
    );
    assert_eq!(ids(&replica.read(read(0, true)).unwrap().page), ["sale"]);
    let before = replica.view().unwrap();
    assert_eq!(writer.resume_cursor().unwrap(), "committed");
    assert!(
        Arc::ptr_eq(&before, &replica.view().unwrap()),
        "ordinary reconnects must not rebuild state"
    );
    writer
        .apply(w::Batch {
            cursor: "committed".into(),
            changes: vec![],
        })
        .unwrap();
    assert_eq!(replica.status().state, SyncState::Live);
}

#[test]
fn publication_failure_recovery_does_not_depend_on_cursor_inequality() {
    for cursor in ["c0", "next"] {
        let (_dir, replica) = setup();
        let mut writer = replica.writer.lock().unwrap();
        writer.mark(SyncState::Stale);
        let before = replica.view().unwrap();
        let revision = replica.read_revision(&read(0, true)).unwrap();
        writer.fail_publish_once = true;
        let error = writer
            .apply(w::Batch {
                cursor: cursor.into(),
                changes: vec![
                    upsert(entity(E::Order(order("listing", "1", None)))),
                    upsert(entity(E::Order(order("sale", "2", Some(10))))),
                ],
            })
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::Unavailable);
        assert!(matches!(
            writer.publication,
            PublicationState::Prepared { .. }
        ));
        assert_eq!(writer.cursor().unwrap(), cursor);
        assert!(Arc::ptr_eq(&before, &replica.view().unwrap()));
        let prepared = match &writer.publication {
            PublicationState::Prepared { view, .. } => view.clone(),
            _ => unreachable!(),
        };
        let revision_before_retry = prepared.revisions.sequence;
        writer.fail_publish_once = true;
        assert!(writer.resume_cursor().is_err());
        assert_eq!(writer.resume_cursor().unwrap(), cursor);
        assert!(Arc::ptr_eq(&prepared, &replica.view().unwrap()));
        assert_eq!(
            replica.view().unwrap().revisions.sequence,
            revision_before_retry
        );
        assert!(matches!(writer.publication, PublicationState::Current));
        assert_eq!(replica.status().state, SyncState::Stale);
        assert_eq!(
            ids(&replica.read(read(1, false)).unwrap().page),
            ["listing"]
        );
        assert_eq!(ids(&replica.read(read(0, true)).unwrap().page), ["sale"]);
        assert_ne!(revision, replica.read_revision(&read(0, true)).unwrap());
        let repaired = replica.view().unwrap();
        writer
            .apply(w::Batch {
                cursor: cursor.into(),
                changes: vec![],
            })
            .unwrap();
        assert_eq!(replica.status().state, SyncState::Live);
        assert!(Arc::ptr_eq(&repaired, &replica.view().unwrap()));
    }
}

#[test]
fn an_ack_or_next_delivery_repairs_a_failed_publication_before_applying() {
    for heartbeat in [true, false] {
        let (_dir, replica) = setup();
        let mut writer = replica.writer.lock().unwrap();
        writer.fail_publish_once = true;
        assert!(
            writer
                .apply(w::Batch {
                    cursor: "c0".into(),
                    changes: vec![upsert(entity(E::Order(order("sale", "2", Some(10)))))],
                })
                .is_err()
        );
        writer
            .apply(w::Batch {
                cursor: if heartbeat { "c0" } else { "next" }.into(),
                changes: if heartbeat {
                    vec![]
                } else {
                    vec![upsert(entity(E::Order(order("listing", "1", None))))]
                },
            })
            .unwrap();
        assert!(matches!(writer.publication, PublicationState::Current));
        assert_eq!(ids(&replica.read(read(0, true)).unwrap().page), ["sale"]);
        assert_eq!(
            replica.read(read(1, false)).unwrap().page.entities.len(),
            usize::from(!heartbeat)
        );
    }
}

#[test]
fn rejected_storage_writes_do_not_rebuild_state_on_resume() {
    let (_dir, replica) = setup();
    apply(
        &replica,
        vec![upsert(entity(E::Order(order("sale", "2", Some(10)))))],
        "saved",
    );
    let before = replica.view().unwrap();
    let changes = replica.changes();
    let mut writer = replica.writer.lock().unwrap();
    for _ in 0..3 {
        assert!(
            writer
                .apply(w::Batch {
                    cursor: "rejected".into(),
                    changes: vec![
                        upsert(entity(E::Order(order("listing", "1", None)))),
                        upsert(entity(E::Order(order("sale", "99", Some(10)))))
                    ],
                })
                .is_err()
        );
        assert!(matches!(writer.publication, PublicationState::Current));
        assert_eq!(writer.resume_cursor().unwrap(), "saved");
        assert!(Arc::ptr_eq(&before, &replica.view().unwrap()));
        assert!(!changes.has_changed().unwrap());
    }
}

#[test]
fn checkpoint_only_publication_retry_keeps_revisions_and_data_notifications_quiet() {
    let (_dir, replica) = setup();
    let mut writer = replica.writer.lock().unwrap();
    writer.mark(SyncState::Stale);
    let changes = replica.changes();
    let revision = replica.read_revision(&w::ReadRequest::default()).unwrap();
    writer.fail_publish_once = true;
    assert!(
        writer
            .apply(w::Batch {
                cursor: "checkpoint".into(),
                changes: vec![]
            })
            .is_err()
    );
    assert_eq!(writer.resume_cursor().unwrap(), "checkpoint");
    assert_eq!(replica.status().state, SyncState::Stale);
    assert_eq!(
        revision,
        replica.read_revision(&w::ReadRequest::default()).unwrap()
    );
    assert!(!changes.has_changed().unwrap());
}

#[test]
fn uncertain_commits_restore_a_consistent_snapshot_even_with_a_repeated_cursor() {
    for committed in [false, true] {
        let (_dir, replica) = setup();
        let mut writer = replica.writer.lock().unwrap();
        writer.mark(SyncState::Stale);
        writer.store.fail_commit_once(committed);
        assert!(
            writer
                .apply(w::Batch {
                    cursor: "c0".into(),
                    changes: vec![
                        upsert(entity(E::Order(order("listing", "1", None)))),
                        upsert(entity(E::Order(order("sale", "2", Some(10)))))
                    ],
                })
                .is_err()
        );
        assert!(matches!(writer.publication, PublicationState::Uncertain));
        assert!(
            replica
                .read(read(1, false))
                .unwrap()
                .page
                .entities
                .is_empty()
        );
        assert_eq!(writer.resume_cursor().unwrap(), "c0");
        assert!(matches!(writer.publication, PublicationState::Current));
        assert_eq!(replica.status().state, SyncState::Stale);
        assert_eq!(
            replica.read(read(1, false)).unwrap().page.entities.len(),
            usize::from(committed)
        );
        assert_eq!(
            replica.read(read(0, true)).unwrap().page.entities.len(),
            usize::from(committed)
        );
        let view = replica.view().unwrap();
        let checkpoint = view.history.checkpoint().unwrap();
        assert_eq!(view.cursor, checkpoint.cursor);
        assert_eq!(view.updated_at, checkpoint.updated_at);
        writer.resume_cursor().unwrap();
        assert!(Arc::ptr_eq(&view, &replica.view().unwrap()));
    }
}

#[test]
fn failed_snapshot_publication_reuses_the_prepared_database_and_persists_future_writes() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "snapshot-retry").unwrap();
    let reader = writer.reader();
    for (name, failed) in [("old", false), ("new", true)] {
        let mut builder = writer.begin_snapshot().unwrap();
        builder.append(&catalog()).unwrap();
        builder
            .append(&[
                upsert(entity(E::Order(order(name, "1", None)))),
                upsert(entity(E::Order(order(
                    &format!("{name}-sale"),
                    "2",
                    Some(10),
                )))),
            ])
            .unwrap();
        writer.fail_publish_once = failed;
        let result = writer.install_snapshot(builder.finish(name).unwrap());
        assert_eq!(result.is_err(), failed);
    }
    assert_eq!(reader.cursor().unwrap(), "old");
    let prepared = match &writer.publication {
        PublicationState::Prepared { view, .. } => view.clone(),
        _ => unreachable!(),
    };
    writer.mark(SyncState::Stale);
    assert_eq!(writer.resume_cursor().unwrap(), "new");
    assert!(Arc::ptr_eq(&prepared, &reader.view().unwrap()));
    assert_eq!(ids(&reader.read(read(1, false)).unwrap().page), ["new"]);
    assert_eq!(ids(&reader.read(read(0, true)).unwrap().page), ["new-sale"]);
    writer
        .apply(w::Batch {
            cursor: "updated".into(),
            changes: vec![upsert(entity(E::Order(order("new", "7", None))))],
        })
        .unwrap();
    drop(prepared);
    drop(reader);
    drop(writer);
    let restored = ReplicaWriter::open(directory.path(), "snapshot-retry")
        .unwrap()
        .reader();
    assert_eq!(restored.cursor().unwrap(), "updated");
    assert_eq!(
        state::order(&restored.read(read(1, false)).unwrap().page.entities[0])
            .unwrap()
            .price,
        "7"
    );
    assert_eq!(
        ids(&restored.read(read(0, true)).unwrap().page),
        ["new-sale"]
    );
}

#[test]
fn pinned_and_retained_reads_keep_the_selected_views_checkpoint_time() {
    let (_dir, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("b", "2", None)))),
        ],
        "facts",
    );
    let selected = replica.view().unwrap();
    let first = replica.read(read(1, false)).unwrap();
    // Model the interleaving where a read selected an old view, then publication
    // advanced status before the read pinned it or completed a retained page.
    let mut status = replica.status();
    status.updated_at = Some(selected.updated_at.unwrap() + 1000);
    replica.reader.0.status.send_replace(status);
    let pinned = replica.pin(selected.clone());
    assert_eq!(pinned.status().updated_at, selected.updated_at);
    assert_eq!(pinned.cursor().unwrap(), selected.cursor);
    let mut continuation = read(1, false);
    continuation.page_token = first.page.next_page_token;
    let result = replica.read(continuation).unwrap();
    assert_eq!(result.sync.updated_at, selected.updated_at);
    assert_eq!(ids(&result.page), ["b"]);
}
fn text(value: &str) -> w::AttributeValue {
    w::AttributeValue {
        value: Some(w::attribute_value::Value::Text(value.into())),
    }
}
fn sticker(name: &str, wear: &str) -> w::AttributeValue {
    w::AttributeValue {
        value: Some(w::attribute_value::Value::Object(w::AttributeObject {
            fields: [
                ("name".into(), text(name)),
                (
                    "wear".into(),
                    w::AttributeValue {
                        value: Some(w::attribute_value::Value::Decimal(wear.into())),
                    },
                ),
            ]
            .into(),
        })),
    }
}
fn eq(value: w::AttributeValue) -> w::AttributePredicate {
    w::AttributePredicate {
        predicate: Some(w::attribute_predicate::Predicate::Eq(value)),
    }
}
fn contains(name: &str, wear: &str, min: u32) -> w::AttributePredicate {
    w::AttributePredicate {
        predicate: Some(w::attribute_predicate::Predicate::Contains(Box::new(
            w::ContainsPredicate {
                min_count: Some(min),
                max_count: None,
                predicate: Some(Box::new(w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Object(
                        w::ObjectPredicate {
                            fields: [
                                ("name".into(), eq(text(name))),
                                (
                                    "wear".into(),
                                    eq(w::AttributeValue {
                                        value: Some(w::attribute_value::Value::Decimal(
                                            wear.into(),
                                        )),
                                    }),
                                ),
                            ]
                            .into(),
                        },
                    )),
                })),
            },
        ))),
    }
}
#[test]
fn history_leaf_indexes_preserve_duplicate_counts_and_same_object_correlation() {
    let (_dir, replica) = setup();
    let mut row = order("stickers", "1", Some(10));
    row.attributes.insert(
        "stickers".into(),
        eq(w::AttributeValue {
            value: Some(w::attribute_value::Value::List(w::AttributeValues {
                values: vec![
                    sticker("gold", "0.1"),
                    sticker("gold", "0.1"),
                    sticker("silver", "0.2"),
                ],
            })),
        }),
    );
    apply(&replica, vec![upsert(entity(E::Order(row)))], "c1");
    let mut count_query = read(0, true);
    if let Some(w::selection::Source::Scope(scope)) =
        &mut count_query.selection.as_mut().unwrap().source
    {
        scope.attributes.insert(
            "stickers".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Contains(Box::new(
                    w::ContainsPredicate {
                        predicate: Some(Box::new(w::AttributePredicate {
                            predicate: Some(w::attribute_predicate::Predicate::Object(
                                w::ObjectPredicate::default(),
                            )),
                        })),
                        min_count: Some(3),
                        max_count: Some(3),
                    },
                ))),
            },
        );
    }
    assert_eq!(
        replica.read(count_query).unwrap().page.entities.len(),
        1,
        "empty object predicates can count every object occurrence"
    );
    for (name, wear, min, expected) in [
        ("gold", "0.10", 2, 1),
        ("gold", "0.1", 3, 0),
        ("gold", "0.2", 1, 0),
        ("missing", "0.1", 0, 1),
    ] {
        let mut query = read(0, true);
        let scope = match query.selection.as_mut().unwrap().source.as_mut().unwrap() {
            w::selection::Source::Scope(s) => s,
            _ => unreachable!(),
        };
        scope
            .attributes
            .insert("stickers".into(), contains(name, wear, min));
        assert_eq!(replica.read(query).unwrap().page.entities.len(), expected);
    }
}

#[test]
fn book_directory_tracks_feed_references_and_price_mode_currency_moves_without_ghosts() {
    let (_dir, replica) = setup();
    let feed = entity(E::Feed(w::Feed {
        id: "feed".into(),
        books: vec![order("a", "1", None).book.unwrap()],
        ..Default::default()
    }));
    apply(
        &replica,
        vec![
            upsert(feed),
            upsert(entity(E::Order(order("a", "1", None)))),
        ],
        "c1",
    );
    assert_eq!(replica.books(None, 10).unwrap().books.len(), 1);
    let mut moved = order("a", "2", None);
    moved.mode = 2;
    moved.book.as_mut().unwrap().currency_id = "XTR".into();
    apply(&replica, vec![upsert(entity(E::Order(moved)))], "c2");
    assert!(
        replica
            .read(read(1, false))
            .unwrap()
            .page
            .entities
            .is_empty()
    );
    assert_eq!(replica.books(None, 10).unwrap().books.len(), 2);
    apply(
        &replica,
        vec![w::Change {
            change: Some(w::change::Change::Remove(w::Removal {
                key: Some(w::Key {
                    kind: 6,
                    id: "feed".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }],
        "c3",
    );
    assert_eq!(replica.books(None, 10).unwrap().books[0].currency_id, "XTR");
    apply(
        &replica,
        vec![w::Change {
            change: Some(w::change::Change::Remove(w::Removal {
                key: Some(w::Key {
                    kind: 7,
                    venue_id: "venue".into(),
                    id: "a".into(),
                }),
                ..Default::default()
            })),
        }],
        "c4",
    );
    assert!(replica.books(None, 10).unwrap().books.is_empty());
}

#[test]
fn shared_book_references_and_directory_pagination_survive_restart() {
    let (directory, replica) = setup();
    let primary = order("a", "1", None).book.unwrap();
    let mut declared_only = primary.clone();
    declared_only.venue_id = "zeta".into();
    let mut bid = order("b", "2", None);
    bid.mode = w::OrderMode::Bid as i32;
    let mut stars = order("stars", "3", None);
    stars.book.as_mut().unwrap().currency_id = "XTR".into();
    apply(
        &replica,
        vec![
            upsert(entity(E::Feed(w::Feed {
                id: "first".into(),
                books: vec![primary.clone(), primary.clone(), declared_only.clone()],
                ..Default::default()
            }))),
            upsert(entity(E::Feed(w::Feed {
                id: "second".into(),
                books: vec![primary.clone()],
                ..Default::default()
            }))),
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(bid))),
            upsert(entity(E::Order(stars))),
        ],
        "books1",
    );
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let page = replica.books(after, 1).unwrap();
        all.extend(page.books);
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(
        all,
        [
            primary.clone(),
            w::BookKey {
                currency_id: "XTR".into(),
                ..primary.clone()
            },
            declared_only
        ]
    );
    let remove = |kind, id: &str| w::Change {
        change: Some(w::change::Change::Remove(w::Removal {
            key: Some(w::Key {
                kind,
                id: id.into(),
                venue_id: if kind == 7 {
                    "venue".into()
                } else {
                    String::new()
                },
            }),
            ..Default::default()
        })),
    };
    apply(
        &replica,
        vec![remove(6, "first"), remove(7, "a"), remove(6, "second")],
        "books2",
    );
    assert_eq!(
        replica.books(None, 10).unwrap().books.len(),
        2,
        "the remaining bid keeps its book alive"
    );
    apply(&replica, vec![remove(7, "b")], "books3");
    let expected = replica.books(None, 10).unwrap();
    assert_eq!(expected.books.len(), 1);
    assert_eq!(expected.books[0].currency_id, "XTR");
    drop(replica);
    let restored = Fixture::open(directory.path(), "test-source").unwrap();
    assert_eq!(restored.books(None, 10).unwrap(), expected);
}

#[test]
fn legacy_continuations_keep_their_shape_and_exact_boundaries() {
    use crate::types::{EntityKind, Key, Position, Sort};
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::json;
    assert_eq!(
        serde_json::to_value(Key::order("venue".into(), "a".into())).unwrap(),
        json!([7, "venue", "a"])
    );
    let position = Position {
        executed_at_ms: 10,
        venue_id: "venue".into(),
        order_id: "x".into(),
    };
    assert_eq!(
        serde_json::to_value(position).unwrap(),
        json!([10, "venue", "x"])
    );
    assert_eq!(serde_json::to_value(Sort::PriceDesc).unwrap(), json!(2));
    assert!(serde_json::from_value::<Key>(json!([0, "venue", "a"])).is_err());
    assert!(serde_json::from_value::<Key>(json!([1, "venue", "a"])).is_err());
    assert!(serde_json::from_value::<Sort>(json!(99)).is_err());
    let (_directory, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("b", "1", None)))),
            upsert(entity(E::Order(order("c", "2", None)))),
            upsert(entity(E::Order(order("x", "1", Some(10))))),
            upsert(entity(E::Order(order("y", "1", Some(20))))),
        ],
        "typed",
    );
    for (history, sort, boundary, expected) in [
        (false, 1, json!({"Price":["1",[7,"venue","a"]]}), "b"),
        (false, 2, json!({"Price":["2",[7,"venue","c"]]}), "b"),
        (false, 0, json!({"Current":[7,"venue","a"]}), "b"),
        (true, 0, json!({"History":[10,"venue","x"]}), "y"),
    ] {
        let mut query = read(1, history);
        query.sort = sort;
        let filter = crate::query::Filter::compile(&replica.view().unwrap().state, &query).unwrap();
        query.page_token = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "selection":filter.fingerprint, "sort":sort, "position":boundary, "snapshot":null
            }))
            .unwrap(),
        );
        assert_eq!(ids(&replica.read(query).unwrap().page), [expected]);
    }
    // Price and time sorts emit canonical cursors, including after a legacy price cursor.
    for sort in 1..=4 {
        let mut request = read(1, false);
        request.sort = sort;
        let first = replica.read(request.clone()).unwrap();
        let mut token: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&first.page.next_page_token).unwrap())
                .unwrap();
        assert!(token["position"].get("Ranked").is_some());
        if sort <= 2 {
            token["position"] = json!({"Price":token["position"]["Ranked"]});
            request.page_token = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&token).unwrap());
            let next = replica.read(request).unwrap();
            let token: serde_json::Value =
                serde_json::from_slice(&URL_SAFE_NO_PAD.decode(next.page.next_page_token).unwrap())
                    .unwrap();
            assert!(token["position"].get("Ranked").is_some());
            assert!(token["position"].get("Price").is_none());
        }
    }
    // Inclusive price bounds must retain every identity tied at the same exact price.
    let mut query = read(1, false);
    query.limit = 10;
    if let Some(w::selection::Source::Scope(scope)) =
        query.selection.as_mut().unwrap().source.as_mut()
    {
        scope.price = Some(w::PriceRange {
            min: Some("1".into()),
            max: Some("1.000".into()),
        });
    }
    assert_eq!(ids(&replica.read(query.clone()).unwrap().page), ["a", "b"]);
    query.sort = 2;
    assert_eq!(ids(&replica.read(query).unwrap().page), ["b", "a"]);
    let mut invalid = read(1, false);
    invalid.selection.as_mut().unwrap().kinds = vec![0];
    assert!(replica.read(invalid).is_err());
    let mut keys = replica
        .view()
        .unwrap()
        .state
        .range(None, None)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    keys.sort();
    assert_eq!(keys.first().unwrap().kind, EntityKind::Venue);
}

#[test]
fn time_number_and_price_sort_whole_books_and_history_with_stable_keyset_pages() {
    let (dir, replica) = setup();
    let numbered = |id: &str, price: &str, time: i64, number: i64, history: bool| {
        let mut order = order(id, price, history.then_some(time));
        order.observed_at_ms = if history { 100 - time } else { time };
        order.attributes.insert(
            "tg.number".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::Integer(number)),
                })),
            },
        );
        order
    };
    let mut changes = Vec::new();
    for history in [false, true] {
        for (id, price, time, number) in [
            ("a", "10000000000000000.000000002", 30, 10),
            ("b", "10000000000000000.000000001", 10, 2),
            ("c", "2", 20, 5),
            ("d", "2", 20, 5),
        ] {
            changes.push(upsert(entity(E::Order(numbered(
                &format!("{}-{id}", if history { "hist" } else { "act" }),
                price,
                time,
                number,
                history,
            )))));
        }
    }
    apply(&replica, changes, "ranked1");
    let collect = |replica: &Fixture, sort: i32, history: bool| {
        let mut query = read(0, history);
        query.sort = sort;
        query.limit = 2;
        let mut result = Vec::new();
        loop {
            let page = replica.read(query.clone()).unwrap().page;
            result.extend(ids(&page));
            if page.next_page_token.is_empty() {
                break;
            }
            query.page_token = page.next_page_token;
        }
        result
    };
    for history in [false, true] {
        for (sort, expected) in [
            (1, vec!["c", "d", "b", "a"]),
            (2, vec!["a", "b", "d", "c"]),
            (3, vec!["b", "c", "d", "a"]),
            (4, vec!["a", "d", "c", "b"]),
            (5, vec!["b", "c", "d", "a"]),
            (6, vec!["a", "d", "c", "b"]),
        ] {
            let expected: Vec<_> = expected
                .into_iter()
                .map(|id| format!("{}-{id}", if history { "hist" } else { "act" }))
                .collect();
            assert_eq!(
                collect(&replica, sort, history),
                expected,
                "sort {sort}, history {history}"
            );
        }
    }
    let mut snapshot = read(0, false);
    snapshot.sort = 4;
    snapshot.limit = 2;
    let first = replica.read(snapshot.clone()).unwrap().page;
    assert_eq!(ids(&first), ["act-a", "act-d"]);
    apply(
        &replica,
        vec![upsert(entity(E::Order(numbered(
            "act-a", "3", 0, 99, false,
        ))))],
        "ranked2",
    );
    snapshot.page_token = first.next_page_token;
    assert_eq!(
        ids(&replica.read(snapshot).unwrap().page),
        ["act-c", "act-b"]
    );
    assert_eq!(
        collect(&replica, 4, false),
        ["act-d", "act-c", "act-b", "act-a"]
    );
    drop(replica);
    let reopened = Fixture::open(dir.path(), "test-source").unwrap();
    assert_eq!(
        collect(&reopened, 6, false),
        ["act-a", "act-d", "act-c", "act-b"]
    );
    assert_eq!(
        collect(&reopened, 4, true),
        ["hist-a", "hist-d", "hist-c", "hist-b"]
    );
}

#[test]
fn expired_views_release_old_roots_without_client_reads() {
    let (_dir, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("b", "2", None)))),
        ],
        "c1",
    );
    let first = replica
        .read(read(w::ReadMode::Snapshot as i32, false))
        .unwrap();
    assert!(!first.page.next_page_token.is_empty());
    let weak = {
        let mut views = replica.reader.0.views.lock().unwrap();
        let retained = views.values_mut().next().unwrap();
        retained.until = Instant::now() - Duration::from_secs(1);
        Arc::downgrade(&retained.view)
    };
    apply(&replica, vec![], "c2"); // no subsequent API read is needed
    assert!(replica.reader.0.views.lock().unwrap().is_empty());
    assert!(weak.upgrade().is_none());
    // Idle maintenance uses the same cleanup, including without a new commit.
    replica
        .read(read(w::ReadMode::Snapshot as i32, false))
        .unwrap();
    replica
        .reader
        .0
        .views
        .lock()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()
        .until = Instant::now() - Duration::from_secs(1);
    replica.writer.lock().unwrap().expire_views();
    assert!(replica.reader.0.views.lock().unwrap().is_empty());
}

#[test]
fn scoped_revisions_survive_coalescing_moves_deletions_and_replacements() {
    let (_dir, replica) = setup();
    let mut other = order("b", "2", None);
    other.book.as_mut().unwrap().currency_id = "XTR".into();
    let a = order("a", "1", None);
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(a.clone()))),
            upsert(entity(E::Order(other.clone()))),
        ],
        "c1",
    );
    let request = read(w::ReadMode::Live as i32, false);
    let revision = replica.reader.read_revision(&request).unwrap();
    let directory = replica.reader.directory_revision().unwrap();
    other.price = "3".into();
    apply(&replica, vec![upsert(entity(E::Order(other)))], "c2");
    assert_eq!(replica.reader.read_revision(&request).unwrap(), revision);
    assert_eq!(replica.reader.directory_revision().unwrap(), directory);
    let mut changed = a.clone();
    changed.price = "4".into();
    apply(
        &replica,
        vec![upsert(entity(E::Order(changed.clone())))],
        "c3",
    );
    apply(&replica, vec![], "c4"); // coalesced notification must not hide c3
    assert_ne!(replica.reader.read_revision(&request).unwrap(), revision);
    assert_eq!(replica.reader.directory_revision().unwrap(), directory);
    let current = replica.reader.read_revision(&request).unwrap();
    apply(
        &replica,
        vec![upsert(entity(E::Order(changed.clone())))],
        "c5",
    );
    assert_eq!(replica.reader.read_revision(&request).unwrap(), current);
    changed.book.as_mut().unwrap().currency_id = "XTR".into();
    apply(&replica, vec![upsert(entity(E::Order(changed)))], "c6");
    assert_ne!(replica.reader.read_revision(&request).unwrap(), current);
    assert_ne!(replica.reader.directory_revision().unwrap(), directory);
    apply(&replica, vec![upsert(entity(E::Order(a)))], "c7");
    let before = replica.reader.read_revision(&request).unwrap();
    let store = replica.writer.lock().unwrap().store.clone();
    replica.install(store).unwrap();
    assert_ne!(replica.reader.read_revision(&request).unwrap(), before);
}

#[tokio::test]
async fn maintenance_releases_expired_snapshots_during_idle_upstream() {
    let (_dir, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("b", "2", None)))),
        ],
        "c1",
    );
    replica
        .read(read(w::ReadMode::Snapshot as i32, false))
        .unwrap();
    replica
        .reader
        .0
        .views
        .lock()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()
        .until = Instant::now() - Duration::from_secs(1);
    // A loopback listener accepts no RPCs; upstream activity cannot do the cleanup.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = market_hub_client::HubClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        "idle.synthetic-test-signature".into(),
    )
    .unwrap();
    let writer = replica.writer.into_inner().unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = crate::ReplicationWorker {
        writer: Arc::new(Mutex::new(writer)),
    };
    let task = tokio::spawn(worker.run(hub, stop.clone()));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if replica.reader.0.views.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    stop.cancel();
    task.await.unwrap().unwrap();
}

#[test]
fn metadata_revisions_track_selected_kinds_deletions_and_dataset_identity() {
    let (_dir, replica) = setup();
    let kinds = [w::EntityKind::Venue, w::EntityKind::Currency];
    let reference = replica.reader.metadata_revision(&kinds).unwrap();
    assert_eq!(
        reference,
        replica
            .reader
            .metadata_revision(&[
                w::EntityKind::Currency,
                w::EntityKind::Venue,
                w::EntityKind::Venue
            ])
            .unwrap()
    );
    let gifts = replica
        .reader
        .metadata_revision(&[w::EntityKind::ItemType, w::EntityKind::AttributeOption])
        .unwrap();
    let directory = replica.reader.directory_revision().unwrap();
    let venue = w::Venue {
        id: "venue".into(),
        name: "Updated".into(),
    };
    apply(
        &replica,
        vec![upsert(entity(E::Venue(venue.clone())))],
        "metadata.1",
    );
    apply(&replica, vec![], "metadata.heartbeat");
    let changed = replica.reader.metadata_revision(&kinds).unwrap();
    assert_ne!(changed, reference);
    assert_eq!(
        gifts,
        replica
            .reader
            .metadata_revision(&[w::EntityKind::ItemType, w::EntityKind::AttributeOption])
            .unwrap()
    );
    assert_eq!(directory, replica.reader.directory_revision().unwrap());
    apply(
        &replica,
        vec![upsert(entity(E::Venue(venue.clone())))],
        "metadata.identical",
    );
    assert_eq!(changed, replica.reader.metadata_revision(&kinds).unwrap());
    apply(
        &replica,
        vec![w::Change {
            change: Some(w::change::Change::Remove(w::Removal {
                key: Some(w::Key {
                    kind: w::EntityKind::Venue as i32,
                    id: "venue".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }],
        "metadata.removed",
    );
    let removed = replica.reader.metadata_revision(&kinds).unwrap();
    assert_ne!(removed, changed);
    apply(
        &replica,
        vec![upsert(entity(E::Venue(venue)))],
        "metadata.restored",
    );
    assert_ne!(removed, replica.reader.metadata_revision(&kinds).unwrap());
    for kind in [
        w::EntityKind::Order,
        w::EntityKind::Feed,
        w::EntityKind::Unspecified,
    ] {
        assert_eq!(
            replica
                .reader
                .metadata_revision(&[kind])
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument
        );
    }
    let before = replica.reader.metadata_revision(&kinds).unwrap();
    let store = replica.writer.lock().unwrap().store.clone();
    replica.install(store).unwrap();
    assert_ne!(before, replica.reader.metadata_revision(&kinds).unwrap());
    assert_ne!(directory, replica.reader.directory_revision().unwrap());
}

#[test]
fn ranked_snapshot_continuations_keep_old_payloads_after_same_rank_updates() {
    let (_dir, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("b", "2", None)))),
        ],
        "shared1",
    );
    let mut continuations = Vec::new();
    for sort in 1..=6 {
        let mut request = read(0, false);
        request.sort = sort;
        let first = replica.read(request.clone()).unwrap().page;
        assert!(!first.next_page_token.is_empty());
        request.page_token = first.next_page_token;
        continuations.push(request);
    }
    let mut a = order("a", "1", None);
    let mut b = order("b", "2", None);
    a.meta.insert("label".into(), "updated".into());
    b.meta.insert("label".into(), "updated".into());
    apply(
        &replica,
        vec![upsert(entity(E::Order(a))), upsert(entity(E::Order(b)))],
        "shared2",
    );
    for request in continuations {
        let pinned = replica.read(request.clone()).unwrap().page;
        assert_eq!(pinned.entities.len(), 1);
        assert!(state::order(&pinned.entities[0]).unwrap().meta.is_empty());
        let mut live = request;
        live.mode = 1;
        let current = replica.read(live).unwrap().page;
        assert_eq!(current.entities.len(), 1);
        assert_eq!(
            state::order(&current.entities[0]).unwrap().meta["label"],
            "updated"
        );
    }
}

fn collect_current_rows(replica: &Fixture, mut request: w::ReadRequest) -> Vec<w::Entity> {
    let mut result = Vec::new();
    for _ in 0..100 {
        let page = replica.read(request.clone()).unwrap().page;
        result.extend(page.entities);
        if page.next_page_token.is_empty() {
            return result;
        }
        request.page_token = page.next_page_token;
    }
    panic!("identity pagination did not terminate");
}

#[test]
fn partition_owned_identity_pages_keep_mixed_snapshots_legacy_cursors_and_restart_order() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let (dir, replica) = setup();
    let mut a = order("same", "1", None);
    a.book.as_mut().unwrap().venue_id = "a".into();
    let mut b = order("same", "2", None);
    b.book.as_mut().unwrap().venue_id = "b".into();
    b.book.as_mut().unwrap().currency_id = "XTR".into();
    b.mode = 2;
    apply(
        &replica,
        vec![
            upsert(entity(E::Venue(w::Venue {
                id: "a".into(),
                name: "A".into(),
            }))),
            upsert(entity(E::Venue(w::Venue {
                id: "b".into(),
                name: "B".into(),
            }))),
            upsert(entity(E::Feed(w::Feed {
                id: "feed".into(),
                books: vec![a.book.clone().unwrap(), b.book.clone().unwrap()],
                ..Default::default()
            }))),
            upsert(entity(E::Order(a.clone()))),
            upsert(entity(E::Order(b.clone()))),
        ],
        "owned1",
    );
    let mixed = w::ReadRequest {
        selection: Some(w::Selection::default()),
        limit: 2,
        sort: 0,
        mode: 0,
        ..Default::default()
    };
    let initial = collect_current_rows(&replica, mixed.clone());
    let first = replica.read(mixed.clone()).unwrap().page;
    let mut pinned_request = mixed.clone();
    pinned_request.page_token = first.next_page_token;
    let mut order_request = w::ReadRequest {
        selection: Some(w::Selection {
            kinds: vec![w::EntityKind::Order as i32],
            ..Default::default()
        }),
        limit: 1,
        sort: 0,
        mode: 1,
        ..Default::default()
    };
    let first_order = replica.read(order_request.clone()).unwrap().page;
    assert_eq!(
        state::entity_key(&first_order.entities[0]).unwrap(),
        state::entity_key(&entity(E::Order(a.clone()))).unwrap()
    );
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(first_order.next_page_token).unwrap())
            .unwrap();
    assert_eq!(
        legacy["position"],
        serde_json::json!({"Current": [7, "a", "same"]})
    );
    legacy["snapshot"] = serde_json::Value::Null;
    order_request.page_token = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&legacy).unwrap());
    a.mode = 2;
    a.price = "9".into();
    a.book.as_mut().unwrap().currency_id = "XTR".into();
    let mut c = order("new", "3", None);
    c.book.as_mut().unwrap().venue_id = "c".into();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(a.clone()))),
            w::Change {
                change: Some(w::change::Change::Remove(w::Removal {
                    key: Some(w::Key {
                        kind: 7,
                        venue_id: "b".into(),
                        id: "same".into(),
                    }),
                    ..Default::default()
                })),
            },
            upsert(entity(E::Order(c))),
            upsert(entity(E::Venue(w::Venue {
                id: "a".into(),
                name: "Changed A".into(),
            }))),
        ],
        "owned2",
    );
    let mut pinned_rows = first.entities;
    pinned_rows.extend(collect_current_rows(&replica, pinned_request));
    assert_eq!(pinned_rows, initial);
    let resumed = collect_current_rows(&replica, order_request);
    assert_eq!(resumed.len(), 1);
    assert_eq!(state::order(&resumed[0]).unwrap().id, "new");
    let mut live = mixed;
    live.mode = 1;
    let current = collect_current_rows(&replica, live.clone());
    let keys: Vec<_> = current
        .iter()
        .map(|e| state::entity_key(e).unwrap())
        .collect();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(current.contains(&entity(E::Order(a))));
    assert!(!current.contains(&entity(E::Order(b))));
    drop(replica);
    let reopened = Fixture::open(dir.path(), "test-source").unwrap();
    assert_eq!(collect_current_rows(&reopened, live), current);
}

#[test]
fn explicit_identity_keys_keep_snapshot_payloads_and_skip_missing_orders_after_moves() {
    let (_dir, replica) = setup();
    let mut a = order("a", "1", None);
    let b = order("b", "2", None);
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(a.clone()))),
            upsert(entity(E::Order(b.clone()))),
        ],
        "keys1",
    );
    let mut request = w::ReadRequest {
        selection: Some(w::Selection {
            keys: vec![
                w::Key {
                    kind: 7,
                    venue_id: "venue".into(),
                    id: "missing".into(),
                },
                w::Key {
                    kind: 7,
                    venue_id: "venue".into(),
                    id: "b".into(),
                },
                w::Key {
                    kind: 1,
                    id: "venue".into(),
                    ..Default::default()
                },
                w::Key {
                    kind: 7,
                    venue_id: "venue".into(),
                    id: "a".into(),
                },
                w::Key {
                    kind: 7,
                    venue_id: "venue".into(),
                    id: "a".into(),
                },
            ],
            ..Default::default()
        }),
        sort: 0,
        mode: 0,
        limit: 1,
        ..Default::default()
    };
    let expected = collect_current_rows(&replica, request.clone());
    assert_eq!(expected.len(), 3);
    let first = replica.read(request.clone()).unwrap().page;
    request.page_token = first.next_page_token;
    a.mode = 2;
    a.book.as_mut().unwrap().currency_id = "XTR".into();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(a.clone()))),
            w::Change {
                change: Some(w::change::Change::Remove(w::Removal {
                    key: Some(w::Key {
                        kind: 7,
                        venue_id: "venue".into(),
                        id: "b".into(),
                    }),
                    ..Default::default()
                })),
            },
        ],
        "keys2",
    );
    let mut pinned = first.entities;
    pinned.extend(collect_current_rows(&replica, request.clone()));
    assert_eq!(pinned, expected);
    request.mode = 1;
    request.page_token.clear();
    let live = collect_current_rows(&replica, request);
    assert_eq!(live.len(), 2);
    assert_eq!(live[1], entity(E::Order(a)));
}
