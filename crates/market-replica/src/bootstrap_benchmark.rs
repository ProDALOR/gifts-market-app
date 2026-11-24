//! Local staging/replay benchmark; no hub or credentials required.
use crate::{ReplicaWriter, benchmarks::fixture};
use market_hub_protocol::wire as w;
use std::time::Instant;

#[test]
#[ignore = "manual bootstrap benchmark"]
fn bootstrap_storage_benchmark() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "bootstrap-benchmark").unwrap();
    let mut builder = writer.begin_snapshot().unwrap();
    let rows = fixture(10_000);
    let start = Instant::now();
    for page in rows.chunks(500) {
        builder.append(page).unwrap();
    }
    let append = start.elapsed();
    let detail = builder.timings();
    let journal = builder.journal();
    let template = rows
        .iter()
        .find_map(|row| match &row.change {
            Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::Order(order)),
            })) => Some(order.clone()),
            _ => None,
        })
        .unwrap();
    let start = Instant::now();
    for index in 0..2000 {
        let mut order = template.clone();
        order.id = format!("stream-{index}");
        order.mode = w::OrderMode::Ask as i32;
        order.executed_at_ms = None;
        journal
            .push(&w::Batch {
                cursor: format!("j{index}"),
                changes: vec![w::Change {
                    change: Some(w::change::Change::Upsert(w::Entity {
                        entity: Some(w::entity::Entity::Order(order)),
                    })),
                }],
            })
            .unwrap();
    }
    let journal_time = start.elapsed();
    let start = Instant::now();
    let mut position = 0;
    let mut checkpoint = "baseline".to_owned();
    while let Some(replayed) = builder.replay_chunk(position).unwrap() {
        checkpoint = replayed.cursor;
        position = replayed.next_position;
    }
    let replay = start.elapsed();
    let start = Instant::now();
    writer
        .install_snapshot(builder.finish(&checkpoint).unwrap())
        .unwrap();
    let finish = start.elapsed();
    assert_eq!(writer.cursor().unwrap(), "j1999");
    println!(
        "bootstrap: 10,000 orders, 500-row pages, 2,000 journal deliveries; append={append:?}; journal={journal_time:?}; replay={replay:?}; finalize_publish={finish:?}"
    );
    println!(
        "snapshot transactions={}; state={:?}; db_wait={:?}; db_write={:?}; db_commit={:?}",
        detail.transactions, detail.state, detail.db_wait, detail.db_write, detail.db_commit
    );
}

/// Reproduce the previous staging loop alongside the current production builder.
#[test]
#[ignore = "manual mixed active/history staging benchmark"]
fn bootstrap_batching_benchmark() {
    let mut rows = fixture(100_000);
    for (i, row) in rows.iter_mut().enumerate() {
        if let Some(w::change::Change::Upsert(w::Entity {
            entity: Some(w::entity::Entity::Order(order)),
        })) = &mut row.change
            && i % 2 == 0
        {
            order.mode = w::OrderMode::Executed as i32;
            order.executed_at_ms = Some(i as i64 + 1);
        }
    }
    let expected = rows
        .iter()
        .filter(|row| {
            !matches!(&row.change,
        Some(w::change::Change::Upsert(w::Entity { entity: Some(w::entity::Entity::Order(order)) }))
            if order.mode == w::OrderMode::Executed as i32)
        })
        .count();
    for (label, batch_rows) in [
        ("legacy_cow_500", 500),
        ("unique_500", 500),
        ("unique_4000", 4_000),
    ] {
        let mut samples = Vec::new();
        for _ in 0..3 {
            let directory = tempfile::tempdir().unwrap();
            if label.starts_with("legacy") {
                let store =
                    crate::storage::Store::open(&directory.path().join("legacy.redb"), "benchmark")
                        .unwrap();
                let mut state = crate::state::State::default();
                let started = Instant::now();
                for page in rows.chunks(batch_rows) {
                    let mut next = state.clone();
                    next.apply(page).unwrap();
                    store.apply(page, None).unwrap();
                    state = next;
                }
                samples.push(started.elapsed());
                assert_eq!(state.range(None, None).count(), expected);
                assert_eq!(
                    store
                        .snapshot()
                        .unwrap()
                        .state()
                        .unwrap()
                        .range(None, None)
                        .count(),
                    expected
                );
            } else {
                let mut writer = ReplicaWriter::open(directory.path(), "benchmark").unwrap();
                let mut builder = writer.begin_snapshot().unwrap();
                let started = Instant::now();
                for page in rows.chunks(batch_rows) {
                    builder.append(page).unwrap();
                }
                samples.push(started.elapsed());
                let detail = builder.timings();
                writer
                    .install_snapshot(builder.finish("benchmark").unwrap())
                    .unwrap();
                assert_eq!(
                    writer
                        .store
                        .snapshot()
                        .unwrap()
                        .state()
                        .unwrap()
                        .range(None, None)
                        .count(),
                    expected
                );
                println!(
                    "{label}: transactions={}; state={:?}; db_wait={:?}; db_write={:?}; db_commit={:?}",
                    detail.transactions,
                    detail.state,
                    detail.db_wait,
                    detail.db_write,
                    detail.db_commit
                );
            }
        }
        samples.sort();
        println!(
            "{label}: 100,000 orders (50% executions); median={:?}; samples={samples:?}",
            samples[1]
        );
    }
}

/// Run alone so the global allocation counters exclude other tests' work.
#[cfg(feature = "profile-allocations")]
#[test]
#[ignore = "manual allocation regression; use --test-threads=1"]
fn execution_replay_allocation_regression() {
    use crate::{state::State, test_support::*};
    use w::entity::Entity as E;

    let mut state = State::default();
    state.apply(&catalog()).unwrap();
    state
        .upsert(entity(E::Order(order("listing", "1", None))))
        .unwrap();
    let mut execution = order("listing", "1", Some(1));
    execution
        .meta
        .insert("payload".into(), "x".repeat(2 * 1024 * 1024));
    let execution = entity(E::Order(execution));
    let key = crate::state::entity_key(&execution).unwrap();
    assert!(state.entity(&key).is_some());
    let change = upsert(execution);
    let region = stats_alloc::Region::new(&stats_alloc::INSTRUMENTED_SYSTEM);
    state.apply([&change]).unwrap();
    let stats = region.change();
    assert!(state.entity(&key).is_none());
    assert!(
        stats.bytes_allocated < 128 * 1024,
        "execution payload must not be cloned into RAM: {stats:?}"
    );
    println!("borrowed execution replay (2 MiB payload): {stats:?}");
}

#[cfg(feature = "profile-allocations")]
#[test]
#[ignore = "manual failed-batch allocation regression; use --test-threads=1"]
fn failed_snapshot_batch_allocation_regression() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "failed-batch").unwrap();
    let rows = fixture(10_000);
    let mut initial = writer.begin_snapshot().unwrap();
    initial.append(&crate::test_support::catalog()).unwrap();
    writer
        .install_snapshot(initial.finish("old").unwrap())
        .unwrap();
    let mut builder = writer.begin_snapshot().unwrap();
    for page in rows.chunks(500) {
        builder.append(page).unwrap();
    }
    let mut rejected = rows.last().unwrap().clone();
    let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(order)),
    })) = &mut rejected.change
    else {
        unreachable!()
    };
    order.id = "rejected-prefix".into();
    let changes = [rejected, w::Change { change: None }];
    let region = stats_alloc::Region::new(&stats_alloc::INSTRUMENTED_SYSTEM);
    assert!(builder.append(&changes).is_err());
    let allocations = region.change();
    assert!(
        allocations.bytes_allocated < 128 * 1024,
        "aborting a failed batch must not rebuild 10,000 staged orders: {allocations:?}"
    );
    assert!(builder.finish("invalid").is_err());
    assert_eq!(writer.cursor().unwrap(), "old");
    println!("failed batch with 10,000 staged orders: {allocations:?}");
}

#[test]
#[ignore = "manual journal transaction benchmark; use --test-threads=1"]
fn journal_batching_benchmark() {
    let template = fixture(1)
        .into_iter()
        .find(|change| {
            matches!(
                &change.change,
                Some(w::change::Change::Upsert(w::Entity {
                    entity: Some(w::entity::Entity::Order(_))
                }))
            )
        })
        .unwrap();
    let batches = (0..2000)
        .map(|i| w::Batch {
            cursor: format!("cursor-{i}"),
            changes: vec![template.clone()],
        })
        .collect::<Vec<_>>();
    for width in [1, 64] {
        let mut samples = Vec::new();
        for _ in 0..3 {
            let directory = tempfile::tempdir().unwrap();
            let mut writer = ReplicaWriter::open(directory.path(), "journal-benchmark").unwrap();
            let builder = writer.begin_snapshot().unwrap();
            let journal = builder.journal();
            let started = Instant::now();
            for batch in batches.chunks(width) {
                journal.push_many(batch).unwrap();
            }
            samples.push(started.elapsed());
            let stats = journal.stats().unwrap();
            assert_eq!(stats.deliveries, 2000);
            assert_eq!(stats.transactions, 2000_u64.div_ceil(width as u64));
            println!(
                "journal width={width}; transactions={}; bytes={}",
                stats.transactions, stats.bytes
            );
        }
        samples.sort();
        println!(
            "journal width={width}; median={:?}; samples={samples:?}",
            samples[1]
        );
    }
}
