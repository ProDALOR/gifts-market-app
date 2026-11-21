//! Isolated live delivery benchmark; no network or credentials required.
use crate::{ReplicaWriter, benchmarks::fixture};
use market_hub_protocol::wire as w;
use std::time::Instant;

#[test]
#[ignore = "manual live-update benchmark; use --test-threads=1"]
fn live_update_benchmark() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "live-benchmark").unwrap();
    let rows = fixture(10_000);
    let mut builder = writer.begin_snapshot().unwrap();
    for page in rows.chunks(500) {
        builder.append(page).unwrap();
    }
    writer
        .install_snapshot(builder.finish("baseline").unwrap())
        .unwrap();
    let heartbeat = w::Batch {
        cursor: "baseline".into(),
        changes: vec![],
    };
    let view = writer.reader.view().unwrap();
    let start = Instant::now();
    #[cfg(feature = "profile-allocations")]
    let region = stats_alloc::Region::new(&stats_alloc::INSTRUMENTED_SYSTEM);
    for _ in 0..1000 {
        writer
            .apply_deliveries(std::slice::from_ref(&heartbeat))
            .unwrap();
    }
    let elapsed = start.elapsed();
    #[cfg(feature = "profile-allocations")]
    println!("1,000 repeated heartbeats: {:?}", region.change());
    println!("1,000 repeated heartbeats: {elapsed:?}");
    assert!(std::sync::Arc::ptr_eq(
        &view,
        &writer.reader.view().unwrap()
    ));
    drop(view);
    let template = rows
        .iter()
        .find_map(|change| match &change.change {
            Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::Order(order)),
            })) => Some(order),
            _ => None,
        })
        .unwrap();
    for (label, size) in [
        ("individual", 1),
        ("grouped", crate::limits::MAX_APPLY_BATCHES),
    ] {
        let batches: Vec<_> = (0..256)
            .map(|index| {
                let mut order = template.clone();
                order.price = (index + 1).to_string();
                w::Batch {
                    cursor: format!("{label}-{index}"),
                    changes: vec![w::Change {
                        change: Some(w::change::Change::Upsert(w::Entity {
                            entity: Some(w::entity::Entity::Order(order)),
                        })),
                    }],
                }
            })
            .collect();
        let start = Instant::now();
        for group in batches.chunks(size) {
            writer.apply_deliveries(group).unwrap();
        }
        println!(
            "256 real updates ({label}, {} commits): {:?}",
            256 / size,
            start.elapsed()
        );
        assert_eq!(writer.reader.cursor().unwrap(), format!("{label}-255"));
    }
}
