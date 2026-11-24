//! Deterministic startup measurements; temporary databases only, without a hub.
use super::*;
use std::hint::black_box;

fn measure<T>(phase: &str, work: impl FnOnce() -> T) -> T {
    #[cfg(feature = "profile-allocations")]
    let region = stats_alloc::Region::new(&stats_alloc::INSTRUMENTED_SYSTEM);
    let started = Instant::now();
    let value = work();
    let elapsed = started.elapsed();
    #[cfg(feature = "profile-allocations")]
    {
        let stats = region.change();
        println!(
            "startup {phase}: elapsed={elapsed:?} allocations={} allocated_bytes={} retained_bytes={}",
            stats.allocations,
            stats.bytes_allocated,
            stats.bytes_allocated as isize - stats.bytes_deallocated as isize
        );
    }
    #[cfg(not(feature = "profile-allocations"))]
    println!("startup {phase}: elapsed={elapsed:?}");
    value
}

#[test]
#[ignore = "manual startup timing/allocation benchmark"]
fn startup_restore_benchmark() {
    let rows = crate::benchmarks::fixture(20_000);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.redb");
    let store = Store::open(&path, "startup-benchmark").unwrap();
    store.apply(&rows, Some("checkpoint")).unwrap();
    drop(store);
    for sample in 0..3 {
        println!("startup sample={sample} orders=20000");
        let store = measure("database_open", || {
            Store::open(&path, "startup-benchmark").unwrap()
        });
        let snapshot = store.snapshot().unwrap();
        measure("decode_only", || {
            let table = snapshot.read.open_table(CURRENT).unwrap();
            let mut count = 0;
            for row in table.iter().unwrap() {
                let (_, payload) = row.unwrap();
                black_box(w::Entity::decode(payload.value()).unwrap());
                count += 1;
            }
            assert_eq!(count, rows.len());
        });
        let state = measure("restore_state", || snapshot.state().unwrap());
        let revisions = measure("revision_indexes", || {
            crate::revisions::Revisions::replaced(None, &state)
        });
        assert_eq!(state.range(None, None).count(), rows.len());
        drop((revisions, state, snapshot, store));
        let writer = measure("writer_open", || {
            crate::ReplicaWriter::open(directory.path(), "startup-benchmark").unwrap()
        });
        assert_eq!(writer.cursor().unwrap(), "checkpoint");
        drop(writer);
    }
}
