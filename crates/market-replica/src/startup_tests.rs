//! Storage startup/upgrade regression tests, always against temporary databases.
use super::*;
use crate::test_support::{entity, order, upsert};
use w::entity::Entity as E;

#[test]
fn clean_validation_reads_committed_schema_without_waiting_for_writer() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("replica.redb"), "source").unwrap();
    store
        .apply(
            &[upsert(entity(E::Order(order("active", "2", None))))],
            Some("checkpoint"),
        )
        .unwrap();
    // A read-only validator must complete even while a write transaction is open.
    let tx = store.0.begin_write().unwrap();
    tx.open_table(META)
        .unwrap()
        .insert("source", b"uncommitted".as_slice())
        .unwrap();
    let database = store.0.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let validator = std::thread::spawn(move || {
        send.send(initialized(&database, "source")).unwrap();
    });
    let result = receive.recv_timeout(Duration::from_secs(5));
    // Release the writer even on failure, so a regression cannot hang the suite.
    tx.abort().unwrap();
    validator.join().unwrap();
    assert!(result.expect("validation waited for a writer").unwrap());
    assert_eq!(
        initialized(&store.0, "other").unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    drop(store);
    let reopened = Store::open(&directory.path().join("replica.redb"), "source").unwrap();
    assert_eq!(reopened.cursor().unwrap(), "checkpoint");
    assert_eq!(reopened.state().unwrap().range(None, None).count(), 1);
}

#[test]
fn missing_history_table_or_marker_rebuilds_ranks_without_losing_checkpoint() {
    for missing_table in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("replica.redb");
        let store = Store::open(&path, "source").unwrap();
        store
            .apply(
                &[upsert(entity(E::Order(order("sale", "2", Some(100)))))],
                Some("checkpoint"),
            )
            .unwrap();
        let tx = store.0.begin_write().unwrap();
        if missing_table {
            tx.delete_table(SORTED).unwrap();
        } else {
            tx.open_table(META)
                .unwrap()
                .remove("execution_ranks_v1")
                .unwrap();
        }
        tx.commit().unwrap();
        assert!(!initialized(&store.0, "source").unwrap());
        drop(store);
        let reopened = Store::open(&path, "source").unwrap();
        assert_eq!(reopened.cursor().unwrap(), "checkpoint");
        let snapshot = reopened.snapshot().unwrap();
        assert_eq!(
            snapshot.read.open_table(EXECUTIONS).unwrap().len().unwrap(),
            1
        );
        assert_eq!(snapshot.read.open_table(SORTED).unwrap().len().unwrap(), 3);
        assert!(initialized(&reopened.0, "source").unwrap());
    }
}

#[test]
fn clean_reopen_does_not_decode_history_and_preserves_journal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.redb");
    let store = Store::open(&path, "source").unwrap();
    let tx = store.0.begin_write().unwrap();
    // Decoding this sentinel would fail; initialized ranks must be reused.
    tx.open_table(EXECUTIONS)
        .unwrap()
        .insert((100, "venue", "sentinel"), b"invalid protobuf".as_slice())
        .unwrap();
    tx.open_table(JOURNAL)
        .unwrap()
        .insert(1, b"pending".as_slice())
        .unwrap();
    tx.commit().unwrap();
    drop(store);
    let reopened = Store::open(&path, "source").unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(
        snapshot
            .read
            .open_table(JOURNAL)
            .unwrap()
            .get(1)
            .unwrap()
            .unwrap()
            .value(),
        b"pending"
    );
    assert_eq!(
        snapshot.read.open_table(EXECUTIONS).unwrap().len().unwrap(),
        1
    );
}

#[test]
fn wrong_source_and_wrong_table_types_are_rejected_without_reinitializing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.redb");
    let store = Store::open(&path, "source").unwrap();
    store.apply(&[], Some("checkpoint")).unwrap();
    drop(store);
    assert_eq!(
        Store::open(&path, "other").err().unwrap().code(),
        tonic::Code::FailedPrecondition
    );
    let store = Store::open(&path, "source").unwrap();
    assert_eq!(store.cursor().unwrap(), "checkpoint");
    let tx = store.0.begin_write().unwrap();
    tx.delete_table(CURRENT).unwrap();
    tx.open_table(TableDefinition::<u64, u64>::new("current"))
        .unwrap()
        .insert(1, 2)
        .unwrap();
    tx.commit().unwrap();
    drop(store);
    assert!(Store::open(&path, "source").is_err());
    let db = Database::create(&path).unwrap();
    assert_eq!(
        db.begin_read()
            .unwrap()
            .open_table(META)
            .unwrap()
            .get("cursor")
            .unwrap()
            .unwrap()
            .value(),
        b"checkpoint"
    );
}

#[test]
fn restore_rejects_identity_mismatch_and_invalid_active_order() {
    for invalid_rank in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("replica.redb"), "source").unwrap();
        let mut value = order("actual", "1", None);
        if invalid_rank {
            value.price = "invalid".into();
        }
        let value = entity(E::Order(value)).encode_to_vec();
        let tx = store.0.begin_write().unwrap();
        tx.open_table(CURRENT)
            .unwrap()
            .insert(
                (7, "venue", if invalid_rank { "actual" } else { "wrong" }),
                value.as_slice(),
            )
            .unwrap();
        tx.commit().unwrap();
        let error = store
            .state()
            .err()
            .expect("invalid record must fail restore");
        assert_eq!(
            error.code(),
            if invalid_rank {
                tonic::Code::InvalidArgument
            } else {
                tonic::Code::DataLoss
            }
        );
    }
}

#[test]
fn legacy_executed_rows_do_not_become_active_orders_on_restore() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("replica.redb"), "source").unwrap();
    let value = entity(E::Order(order("sale", "2", Some(100))));
    let key = state::entity_key(&value).unwrap();
    let tx = store.0.begin_write().unwrap();
    tx.open_table(CURRENT)
        .unwrap()
        .insert(key.stored(), value.encode_to_vec().as_slice())
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(store.state().unwrap().range(None, None).count(), 0);
}

#[test]
fn startup_rejects_missing_identity_or_primary_tables_without_adopting_checkpoint() {
    for missing in [
        "metadata",
        "source",
        "empty_source",
        "current",
        "executions",
        "identities",
        "index",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("replica.redb");
        let store = Store::open(&path, "source").unwrap();
        store
            .apply(
                &[
                    upsert(entity(E::Order(order("active", "2", None)))),
                    upsert(entity(E::Order(order("sale", "3", Some(100))))),
                ],
                Some("checkpoint"),
            )
            .unwrap();
        let tx = store.0.begin_write().unwrap();
        match missing {
            "metadata" => {
                tx.delete_table(META).unwrap();
            }
            "source" => {
                tx.open_table(META).unwrap().remove("source").unwrap();
            }
            "empty_source" => {
                tx.open_table(META)
                    .unwrap()
                    .insert("source", b"".as_slice())
                    .unwrap();
            }
            "current" => {
                tx.delete_table(CURRENT).unwrap();
            }
            "executions" => {
                tx.delete_table(EXECUTIONS).unwrap();
            }
            "identities" => {
                tx.delete_table(IDENTITIES).unwrap();
            }
            "index" => {
                tx.delete_table(INDEX).unwrap();
            }
            _ => unreachable!(),
        }
        tx.commit().unwrap();
        drop(store);
        let error = Store::open(&path, "source")
            .err()
            .expect("incomplete saved data must not be silently adopted");
        assert_eq!(error.code(), tonic::Code::DataLoss, "missing {missing}");
        let db = Database::create(&path).unwrap();
        let read = db.begin_read().unwrap();
        if missing == "empty_source" {
            assert!(
                read.open_table(META)
                    .unwrap()
                    .get("source")
                    .unwrap()
                    .unwrap()
                    .value()
                    .is_empty()
            );
        } else if missing == "source" {
            assert!(
                read.open_table(META)
                    .unwrap()
                    .get("source")
                    .unwrap()
                    .is_none()
            );
        } else {
            assert!(
                !read
                    .list_tables()
                    .unwrap()
                    .any(|table| table.name() == missing)
            );
        }
        if missing != "metadata" {
            assert_eq!(
                read.open_table(META)
                    .unwrap()
                    .get("cursor")
                    .unwrap()
                    .unwrap()
                    .value(),
                b"checkpoint"
            );
        }
    }
}

#[test]
fn rank_rebuild_replaces_stale_entries_and_rolls_back_on_invalid_history() {
    for invalid in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("replica.redb");
        let store = Store::open(&path, "source").unwrap();
        store
            .apply(
                &[upsert(entity(E::Order(order("sale", "2", Some(100)))))],
                Some("checkpoint"),
            )
            .unwrap();
        let tx = store.0.begin_write().unwrap();
        tx.open_table(META)
            .unwrap()
            .remove("execution_ranks_v1")
            .unwrap();
        let stale_rank = crate::ranking::ordered_decimal(rust_decimal::Decimal::ONE);
        tx.open_table(SORTED)
            .unwrap()
            .insert(
                (
                    0,
                    "gift",
                    "GRAM",
                    stale_rank.as_slice(),
                    (100, "venue", "sale"),
                ),
                (),
            )
            .unwrap();
        if invalid {
            tx.open_table(EXECUTIONS)
                .unwrap()
                .insert((200, "venue", "bad"), b"invalid protobuf".as_slice())
                .unwrap();
        }
        tx.commit().unwrap();
        drop(store);
        if invalid {
            assert!(Store::open(&path, "source").is_err());
            let db = Database::create(&path).unwrap();
            let read = db.begin_read().unwrap();
            assert_eq!(
                read.open_table(SORTED).unwrap().len().unwrap(),
                4,
                "repair failure must roll back removal and rebuild"
            );
            assert!(
                read.open_table(META)
                    .unwrap()
                    .get("execution_ranks_v1")
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                read.open_table(META)
                    .unwrap()
                    .get("cursor")
                    .unwrap()
                    .unwrap()
                    .value(),
                b"checkpoint"
            );
        } else {
            let reopened = Store::open(&path, "source").unwrap();
            let snapshot = reopened.snapshot().unwrap();
            assert_eq!(snapshot.read.open_table(SORTED).unwrap().len().unwrap(), 3);
            assert_eq!(reopened.cursor().unwrap(), "checkpoint");
        }
    }
}

#[test]
fn rank_rebuild_rejects_mismatched_execution_identity() {
    for invalid in ["id", "venue", "time", "mode"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("replica.redb");
        let store = Store::open(&path, "source").unwrap();
        let mut payload = order("actual", "2", Some(100));
        match invalid {
            "id" => payload.id = "wrong".into(),
            "venue" => payload.book.as_mut().unwrap().venue_id = "wrong".into(),
            "time" => payload.executed_at_ms = Some(200),
            "mode" => payload.mode = w::OrderMode::Ask as i32,
            _ => unreachable!(),
        }
        let tx = store.0.begin_write().unwrap();
        tx.open_table(META)
            .unwrap()
            .remove("execution_ranks_v1")
            .unwrap();
        tx.open_table(EXECUTIONS)
            .unwrap()
            .insert((100, "venue", "actual"), payload.encode_to_vec().as_slice())
            .unwrap();
        tx.commit().unwrap();
        drop(store);
        assert_eq!(
            Store::open(&path, "source")
                .err()
                .expect("execution identity mismatch")
                .code(),
            tonic::Code::DataLoss,
            "invalid {invalid}"
        );
    }
}

#[test]
fn restore_uses_one_pinned_transaction_for_rows_and_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("replica.redb"), "source").unwrap();
    store
        .apply(
            &[upsert(entity(E::Order(order("active", "2", None))))],
            Some("before"),
        )
        .unwrap();
    let pinned = store.snapshot().unwrap();
    let writer = store.clone();
    std::thread::spawn(move || {
        writer
            .apply(
                &[upsert(entity(E::Order(order("active", "9", None))))],
                Some("after"),
            )
            .unwrap()
    })
    .join()
    .unwrap();
    let old = pinned.state().unwrap();
    let current = store.state().unwrap();
    let key = state::Key::order("venue".into(), "active".into());
    assert_eq!(pinned.checkpoint().unwrap().cursor, "before");
    assert_eq!(state::order(old.entity(&key).unwrap()).unwrap().price, "2");
    assert_eq!(store.cursor().unwrap(), "after");
    assert_eq!(
        state::order(current.entity(&key).unwrap()).unwrap().price,
        "9"
    );
}

#[test]
fn missing_journal_repairs_without_rescanning_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.redb");
    let store = Store::open(&path, "source").unwrap();
    store.apply(&[], Some("checkpoint")).unwrap();
    let tx = store.0.begin_write().unwrap();
    tx.delete_table(JOURNAL).unwrap();
    // Any attempt to rebuild ready history would fail on this sentinel.
    tx.open_table(EXECUTIONS)
        .unwrap()
        .insert((100, "venue", "sentinel"), b"invalid protobuf".as_slice())
        .unwrap();
    tx.commit().unwrap();
    drop(store);
    let reopened = Store::open(&path, "source").unwrap();
    assert_eq!(reopened.cursor().unwrap(), "checkpoint");
    assert_eq!(
        reopened
            .snapshot()
            .unwrap()
            .read
            .open_table(JOURNAL)
            .unwrap()
            .len()
            .unwrap(),
        0
    );
    assert!(initialized(&reopened.0, "source").unwrap());
}

#[test]
fn rank_repair_does_not_delete_an_incompatible_table() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.redb");
    let store = Store::open(&path, "source").unwrap();
    let incompatible = TableDefinition::<u64, u64>::new(SORTED.name());
    let tx = store.0.begin_write().unwrap();
    tx.open_table(META)
        .unwrap()
        .remove("execution_ranks_v1")
        .unwrap();
    tx.delete_table(SORTED).unwrap();
    tx.open_table(incompatible).unwrap().insert(1, 2).unwrap();
    tx.commit().unwrap();
    drop(store);
    assert!(Store::open(&path, "source").is_err());
    let db = Database::create(&path).unwrap();
    assert_eq!(
        db.begin_read()
            .unwrap()
            .open_table(incompatible)
            .unwrap()
            .get(1)
            .unwrap()
            .unwrap()
            .value(),
        2
    );
}

#[test]
fn empty_source_is_rejected_before_creating_storage() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.redb");
    assert_eq!(
        Store::open(&path, "").err().unwrap().code(),
        tonic::Code::InvalidArgument
    );
    assert!(!path.exists());
}

#[test]
fn cancelled_rank_rebuild_rolls_back_and_can_be_retried() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.redb");
    let store = Store::open(&path, "source").unwrap();
    store
        .apply(
            &[
                upsert(entity(E::Order(order("one", "2", Some(100))))),
                upsert(entity(E::Order(order("two", "3", Some(200))))),
            ],
            Some("saved"),
        )
        .unwrap();
    let tx = store.0.begin_write().unwrap();
    tx.open_table(META)
        .unwrap()
        .remove("execution_ranks_v1")
        .unwrap();
    tx.commit().unwrap();
    drop(store);
    let calls = std::cell::Cell::new(0);
    let error = Store::open_checked(&path, "source", &|| {
        let n = calls.get();
        calls.set(n + 1);
        if n == 4 {
            Err(Status::cancelled("test cancellation"))
        } else {
            Ok(())
        }
    })
    .err()
    .unwrap();
    assert_eq!(error.code(), tonic::Code::Cancelled);
    let db = Database::create(&path).unwrap();
    let read = db.begin_read().unwrap();
    assert_eq!(read.open_table(SORTED).unwrap().len().unwrap(), 6);
    assert!(
        read.open_table(META)
            .unwrap()
            .get("execution_ranks_v1")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        read.open_table(META)
            .unwrap()
            .get("cursor")
            .unwrap()
            .unwrap()
            .value(),
        b"saved"
    );
    drop((read, db));
    let reopened = Store::open(&path, "source").unwrap();
    assert!(initialized(&reopened.0, "source").unwrap());
    assert_eq!(reopened.cursor().unwrap(), "saved");
}

#[test]
fn cancelled_state_restore_discards_partial_indexes_without_changing_storage() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("replica.redb"), "source").unwrap();
    let rows = crate::benchmarks::fixture(128);
    store.apply(&rows, Some("saved")).unwrap();
    let snapshot = store.snapshot().unwrap();
    let calls = std::cell::Cell::new(0);
    let result = snapshot.state_checked(&|| {
        let n = calls.get();
        calls.set(n + 1);
        if n == 40 {
            Err(Status::cancelled("test cancellation"))
        } else {
            Ok(())
        }
    });
    assert_eq!(result.err().unwrap().code(), tonic::Code::Cancelled);
    assert_eq!(snapshot.checkpoint().unwrap().cursor, "saved");
    assert_eq!(
        snapshot.state().unwrap().range(None, None).count(),
        rows.len()
    );
}
