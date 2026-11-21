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
