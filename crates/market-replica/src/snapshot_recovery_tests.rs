//! Replacement failure boundaries and real process exits without Rust destructors.
use crate::test_support::{catalog, entity, ids, order, order_entity, read, upsert};
use crate::{PublicationState, ReplicaReader, ReplicaWriter, SnapshotBuilder, SyncState};
use market_hub_protocol::wire as w;
use std::{path::Path, sync::Arc};

fn build(writer: &mut ReplicaWriter, name: &str) -> SnapshotBuilder {
    let mut builder = writer.begin_snapshot().unwrap();
    let mut rows = catalog();
    for row in &mut rows {
        if let Some(w::change::Change::Upsert(w::Entity {
            entity: Some(w::entity::Entity::ItemType(item)),
        })) = &mut row.change
        {
            item.name = name.into();
        }
    }
    rows.extend([
        upsert(entity(w::entity::Entity::Order(order(
            &format!("{name}-ask"),
            "1",
            None,
        )))),
        upsert(entity(w::entity::Entity::Order(order(
            &format!("{name}-sale"),
            "2",
            Some(10),
        )))),
    ]);
    builder.append(&rows).unwrap();
    builder
}

fn install(writer: &mut ReplicaWriter, name: &str) {
    let snapshot = build(writer, name)
        .finish(&format!("{name}-cursor"))
        .unwrap();
    writer.install_snapshot(snapshot).unwrap();
}

fn verify(reader: &ReplicaReader, name: &str, cursor: &str, price: &str) {
    let view = reader.view().unwrap();
    let checkpoint = view.history.checkpoint().unwrap();
    assert_eq!(view.cursor, cursor);
    assert_eq!(checkpoint.cursor, cursor);
    assert_eq!(checkpoint.updated_at, view.updated_at);
    assert!(checkpoint.updated_at.is_some());
    let asks = reader.read(read(1, false)).unwrap();
    assert_eq!(ids(&asks.page), [format!("{name}-ask")]);
    assert_eq!(order_entity(&asks.page.entities[0]).unwrap().price, price);
    assert_eq!(
        ids(&reader.read(read(0, true)).unwrap().page),
        [format!("{name}-sale")]
    );
    let Some(w::entity::Entity::ItemType(item)) =
        view.state.get(crate::types::EntityKind::ItemType, "gift")
    else {
        panic!("missing catalog")
    };
    assert_eq!(item.name, name);
}

#[test]
fn directory_sync_failure_retains_the_new_database_and_blocks_further_writes() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "replacement-test").unwrap();
    install(&mut writer, "old");
    let reader = writer.reader();
    let pinned = reader.snapshot().unwrap();
    let old_store = writer.store.clone();
    let snapshot = build(&mut writer, "new").finish("new-cursor").unwrap();
    writer.fail_directory_sync_once = true;
    assert!(writer.install_snapshot(snapshot).is_err());
    assert!(!directory.path().join("building.redb").exists());
    let pending = match &writer.publication {
        PublicationState::Prepared {
            view,
            directory_sync: Some(_),
            ..
        } => view.clone(),
        _ => panic!("renamed database and directory barrier must be retained"),
    };
    verify(&reader, "old", "old-cursor", "1");
    // The prepared view owns the renamed file's lock, even while the old view
    // is served. A second writer must not acquire the canonical database.
    assert!(ReplicaWriter::open(directory.path(), "replacement-test").is_err());
    writer.fail_directory_sync_once = true;
    assert!(
        writer.begin_snapshot().is_err(),
        "a new build cannot bypass the replacement barrier"
    );
    assert!(!directory.path().join("building.redb").exists());
    writer.fail_directory_sync_once = true;
    assert!(
        writer
            .apply(w::Batch {
                cursor: "blocked".into(),
                changes: vec![upsert(entity(w::entity::Entity::Order(order(
                    "new-ask", "7", None
                ))))],
            })
            .is_err()
    );
    assert_eq!(old_store.cursor().unwrap(), "old-cursor");
    assert_eq!(pending.history.checkpoint().unwrap().cursor, "new-cursor");
    verify(&reader, "old", "old-cursor", "1");

    writer.mark(SyncState::Stale);
    writer.fail_publish_once = true;
    assert!(writer.resume_cursor().is_err());
    assert!(matches!(
        writer.publication,
        PublicationState::Prepared {
            directory_sync: None,
            ..
        }
    ));
    // A completed barrier is not repeated after a later publication failure.
    writer.fail_directory_sync_once = true;
    assert_eq!(writer.resume_cursor().unwrap(), "new-cursor");
    assert!(writer.fail_directory_sync_once);
    writer.fail_directory_sync_once = false;
    assert!(Arc::ptr_eq(&pending, &reader.view().unwrap()));
    assert_eq!(reader.status().state, SyncState::Stale);
    verify(&reader, "new", "new-cursor", "1");
    verify(&pinned, "old", "old-cursor", "1");
    writer
        .apply(w::Batch {
            cursor: "live-cursor".into(),
            changes: vec![upsert(entity(w::entity::Entity::Order(order(
                "new-ask", "7", None,
            ))))],
        })
        .unwrap();
    assert_eq!(
        old_store.cursor().unwrap(),
        "old-cursor",
        "live writes must not target the unlinked database"
    );
    drop(pending);
    drop(reader);
    drop(writer);
    let restored = ReplicaWriter::open(directory.path(), "replacement-test").unwrap();
    verify(&restored.reader(), "new", "live-cursor", "7");
    // Pinned readers of the old inode remain usable while the replacement is
    // opened and updated; their Store handles do not point at the new file.
    verify(&pinned, "old", "old-cursor", "1");
}

#[test]
fn rename_failure_preserves_the_original_database_and_live_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "rename-failure").unwrap();
    install(&mut writer, "old");
    let snapshot = build(&mut writer, "new").finish("new-cursor").unwrap();
    std::fs::remove_file(directory.path().join("building.redb")).unwrap();
    assert!(writer.install_snapshot(snapshot).is_err());
    assert!(matches!(writer.publication, PublicationState::Current));
    verify(&writer.reader(), "old", "old-cursor", "1");
    writer
        .apply(w::Batch {
            cursor: "old-live".into(),
            changes: vec![],
        })
        .unwrap();
    drop(writer);
    let restored = ReplicaWriter::open(directory.path(), "rename-failure").unwrap();
    verify(&restored.reader(), "old", "old-live", "1");
}

#[test]
fn directory_writer_lock_survives_database_replacements_and_needs_no_stale_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "writer-lock").unwrap();
    let contender = std::fs::File::options()
        .read(true)
        .write(true)
        .open(directory.path().join("replica.lock"))
        .unwrap();
    for index in 0..4 {
        install(&mut writer, &format!("generation-{index}"));
        assert!(
            contender.try_lock().is_err(),
            "directory admission must stay locked across rename"
        );
    }
    let building = directory.path().join("building.redb");
    std::fs::write(&building, b"unpublished sentinel").unwrap();
    assert!(ReplicaWriter::open(directory.path(), "writer-lock").is_err());
    assert_eq!(std::fs::read(&building).unwrap(), b"unpublished sentinel");
    drop(writer);
    assert!(
        contender.try_lock().is_ok(),
        "dropping the writer releases the OS lock"
    );
    contender.unlock().unwrap();
    assert!(directory.path().join("replica.lock").exists());
    let restored = ReplicaWriter::open(directory.path(), "writer-lock").unwrap();
    verify(
        &restored.reader(),
        "generation-3",
        "generation-3-cursor",
        "1",
    );
    assert!(!building.exists());
}

#[test]
fn concurrent_readers_observe_matching_catalog_orders_history_and_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "concurrent-replacement").unwrap();
    install(&mut writer, "generation-0");
    let reader = writer.reader();
    let (stop, stopped) = std::sync::mpsc::channel::<()>();
    let (started, running) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut started = Some(started);
        let mut reads = 0;
        while matches!(
            stopped.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ) {
            let pinned = reader.snapshot().unwrap();
            let cursor = pinned.cursor().unwrap();
            let name = cursor.strip_suffix("-cursor").unwrap();
            verify(&pinned, name, &cursor, "1");
            reads += 1;
            if let Some(started) = started.take() {
                started.send(()).unwrap();
            }
        }
        reads
    });
    running.recv().unwrap();
    for index in 1..9 {
        install(&mut writer, &format!("generation-{index}"));
    }
    drop(stop);
    assert!(worker.join().unwrap() > 0);
    verify(&writer.reader(), "generation-8", "generation-8-cursor", "1");
}

#[test]
fn process_crashes_recover_a_complete_old_or_new_replica_at_every_boundary() {
    for phase in [
        "building",
        "finalized",
        "directory-sync",
        "publication",
        "published",
        "live",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "snapshot_recovery_tests::crash_child",
                "--test-threads=1",
            ])
            .env("MARKET_REPLICA_CRASH_PHASE", phase)
            .env("MARKET_REPLICA_CRASH_DIRECTORY", directory.path())
            .output()
            .unwrap();
        assert_eq!(
            child.status.code(),
            Some(86),
            "{phase}: stdout={} stderr={}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        let writer = ReplicaWriter::open(directory.path(), "crash-test").unwrap();
        assert_eq!(writer.reader().status().state, SyncState::Stale);
        let expected = if matches!(phase, "building" | "finalized") {
            "old"
        } else {
            "new"
        };
        let cursor = if phase == "live" {
            "live-cursor".to_owned()
        } else {
            format!("{expected}-cursor")
        };
        verify(
            &writer.reader(),
            expected,
            &cursor,
            if phase == "live" { "7" } else { "1" },
        );
        assert!(!directory.path().join("building.redb").exists());
    }
}

#[test]
#[ignore = "subprocess helper; called by the crash-recovery test"]
fn crash_child() {
    let Ok(phase) = std::env::var("MARKET_REPLICA_CRASH_PHASE") else {
        return;
    };
    let directory = std::env::var_os("MARKET_REPLICA_CRASH_DIRECTORY").unwrap();
    let mut writer = ReplicaWriter::open(Path::new(&directory), "crash-test").unwrap();
    install(&mut writer, "old");
    let builder = build(&mut writer, "new");
    if phase == "building" {
        std::process::exit(86);
    }
    let snapshot = builder.finish("new-cursor").unwrap();
    if phase == "finalized" {
        std::process::exit(86);
    }
    writer.fail_directory_sync_once = phase == "directory-sync";
    writer.fail_publish_once = phase == "publication";
    let result = writer.install_snapshot(snapshot);
    assert_eq!(
        result.is_err(),
        matches!(phase.as_str(), "directory-sync" | "publication")
    );
    if phase == "live" {
        writer
            .apply(w::Batch {
                cursor: "live-cursor".into(),
                changes: vec![upsert(entity(w::entity::Entity::Order(order(
                    "new-ask", "7", None,
                ))))],
            })
            .unwrap();
    }
    // Exit deliberately skips all Rust destructors, including redb shutdown.
    // This models a process crash, not a loss of power to the filesystem.
    std::process::exit(86);
}
