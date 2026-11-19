//! Repeatable query-path benchmark; run latency and allocation profiles separately.
use super::*;
use std::{hint::black_box, time::Instant};
#[test]
#[ignore = "manual query benchmark; see docs/ordered-query-benchmark.md"]
fn ordered_query_benchmark() {
    let rows = crate::benchmarks::fixture(40_000);
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "query-benchmark").unwrap();
    let mut builder = writer.begin_snapshot().unwrap();
    builder.append(&rows).unwrap();
    writer
        .install_snapshot(builder.finish("bench").unwrap())
        .unwrap();
    let reader = writer.reader();
    let query = w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(w::Scope {
                item_type_id: Some("gift0".into()),
                currency_id: Some("GRAM".into()),
                modes: vec![1],
                ..Default::default()
            })),
            ..Default::default()
        }),
        mode: 1,
        sort: 1,
        ..Default::default()
    };
    let seed = SliceRequest {
        navigation: Navigation::Head,
        limit: 200,
        snapshot: None,
    };
    let first = reader
        .read_slice_checked(query.clone(), seed.clone(), |_, _| Ok(()), |_| Ok(true))
        .unwrap();
    assert_eq!(first.entries.len(), 200);
    println!(
        "{}",
        serde_json::json!({"case":"ordering_keys_200","bytes":first.entries.iter().map(|row| row.key.encode().len()).sum::<usize>()})
    );
    let range = KeyRange {
        start: None,
        end: Some(first.entries.last().unwrap().key.after()),
    };
    for (name, bounds, encode_keys) in [
        ("head_200", None, false),
        ("range_200", Some(range.clone()), false),
        ("head_200_encoded", None, true),
        ("range_200_encoded", Some(range), true),
    ] {
        let mut samples = vec![];
        for i in 0..72 {
            let start = Instant::now();
            let result = if let Some(range) = &bounds {
                reader.read_range_checked(query.clone(), range.clone(), |_, _| Ok(()), |_| Ok(true))
            } else {
                reader.read_slice_checked(query.clone(), seed.clone(), |_, _| Ok(()), |_| Ok(true))
            };
            let result = black_box(result.unwrap());
            assert_eq!(result.entries.len(), 200);
            if encode_keys {
                for row in &result.entries {
                    black_box(row.key.encode());
                }
            }
            if i >= 8 {
                samples.push(start.elapsed().as_nanos());
            }
        }
        samples.sort();
        println!(
            "{}",
            serde_json::json!({"case":name,"median_us":samples[samples.len()/2] as f64/1000.0})
        );
    }
}
