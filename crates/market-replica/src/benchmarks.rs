//! Manual, deterministic before/after benchmark. Latency and allocator profiling use separate builds.
use super::*;
use std::hint::black_box;
#[cfg(not(feature = "profile-allocations"))]
use std::time::Instant;
#[cfg(feature = "profile-allocations")]
#[global_allocator]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

fn entity(value: w::entity::Entity) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Upsert(w::Entity {
            entity: Some(value),
        })),
    }
}
fn order_mut(change: &mut w::Change) -> &mut w::Order {
    match &mut change.change {
        Some(w::change::Change::Upsert(w::Entity {
            entity: Some(w::entity::Entity::Order(order)),
        })) => order,
        _ => unreachable!(),
    }
}
fn predicate(value: w::attribute_value::Value) -> w::AttributePredicate {
    w::AttributePredicate {
        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
            value: Some(value),
        })),
    }
}
pub(super) fn fixture(count: usize) -> Vec<w::Change> {
    let mut rows = Vec::new();
    for i in 0..8 {
        rows.push(entity(w::entity::Entity::Venue(w::Venue {
            id: format!("v{i}"),
            name: format!("Venue {i}"),
        })));
    }
    for id in ["GRAM", "XTR"] {
        rows.push(entity(w::entity::Entity::Currency(w::Currency {
            id: id.into(),
            name: id.into(),
            decimals: 9,
        })));
    }
    rows.push(entity(w::entity::Entity::Attribute(w::Attribute {
        id: "tg.model".into(),
        name: "Model".into(),
        value_type: w::AttributeType::Enum as i32,
        ..Default::default()
    })));
    for i in 0..16 {
        rows.push(entity(w::entity::Entity::AttributeOption(
            w::AttributeOption {
                id: format!("m{i}"),
                attribute_id: "tg.model".into(),
                name: format!("Model {i}"),
                ..Default::default()
            },
        )));
    }
    for i in 0..2 {
        rows.push(entity(w::entity::Entity::ItemType(w::ItemType {
            id: format!("gift{i}"),
            name: format!("Gift {i}"),
            quantity_step: "1".into(),
            attributes: [("tg.model".into(), w::AttributeRule::default())].into(),
            ..Default::default()
        })));
    }
    for i in 0..count {
        let n = mix(i as u64 + 1);
        rows.push(entity(w::entity::Entity::Order(w::Order {
            id: format!("order.{i:06}"),
            book: Some(w::BookKey {
                venue_id: format!("v{}", n % 8),
                item_type_id: format!("gift{}", (n >> 4) % 2),
                currency_id: if (n >> 5).is_multiple_of(2) {
                    "GRAM"
                } else {
                    "XTR"
                }
                .into(),
            }),
            mode: if (n >> 6).is_multiple_of(4) { 2 } else { 1 },
            price: format!("{}.000000001", (n >> 9) % 10_000 + 1),
            quantity: "1".into(),
            observed_at_ms: ((n >> 24) % 1_000_000) as i64,
            attributes: [
                (
                    "tg.model".into(),
                    predicate(w::attribute_value::Value::OptionId(format!(
                        "m{}",
                        (n >> 40) % 16
                    ))),
                ),
                (
                    "tg.number".into(),
                    predicate(w::attribute_value::Value::Integer(i as i64 + 1)),
                ),
            ]
            .into(),
            meta: [("url".into(), format!("https://example.invalid/gifts/{i}"))].into(),
            ..Default::default()
        })));
    }
    rows
}
fn mix(mut n: u64) -> u64 {
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
    n ^ (n >> 31)
}
fn query(sort: w::ReadSort, venue: Option<&str>, sparse: bool) -> w::ReadRequest {
    w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(w::Scope {
                venue_id: venue.map(str::to_owned),
                item_type_id: Some("gift0".into()),
                currency_id: Some("GRAM".into()),
                modes: vec![1],
                attributes: if sparse {
                    [(
                        "tg.model".into(),
                        predicate(w::attribute_value::Value::OptionId("m0".into())),
                    )]
                    .into()
                } else {
                    Default::default()
                },
                ..Default::default()
            })),
            ..Default::default()
        }),
        sort: sort as i32,
        mode: w::ReadMode::Live as i32,
        limit: 200,
        ..Default::default()
    }
}
fn measure(label: &str, name: &str, iterations: usize, mut work: impl FnMut()) {
    for _ in 0..4 {
        work();
    }
    #[cfg(not(feature = "profile-allocations"))]
    {
        let mut ns = Vec::new();
        for _ in 0..9 {
            let start = Instant::now();
            for _ in 0..iterations {
                work();
            }
            ns.push(start.elapsed().as_nanos() as f64 / iterations as f64);
        }
        ns.sort_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({"label":label,"case":name,"metric":"latency","iterations":iterations,"samples":9,"median_ns":ns[4],"min_ns":ns[0],"max_ns":ns[8]})
        );
    }
    #[cfg(feature = "profile-allocations")]
    {
        let region = stats_alloc::Region::new(ALLOCATOR);
        for _ in 0..iterations {
            work();
        }
        let stats = region.change();
        println!(
            "{}",
            serde_json::json!({"label":label,"case":name,"metric":"allocations","iterations":iterations,"allocations_per_op":stats.allocations as f64/iterations as f64,"allocated_bytes_per_op":stats.bytes_allocated as f64/iterations as f64,"reallocations_per_op":stats.reallocations as f64/iterations as f64})
        );
    }
}
#[test]
#[ignore = "manual release benchmark; see docs/ranked-index-benchmark.md"]
fn orderbooks_benchmark() {
    let count = std::env::var("MARKET_BENCH_ORDERS")
        .map(|s| s.parse().unwrap())
        .unwrap_or(40_000);
    let iterations = std::env::var("MARKET_BENCH_ITERATIONS")
        .map(|s| s.parse().unwrap())
        .unwrap_or(32);
    assert!(iterations > 0, "MARKET_BENCH_ITERATIONS must be positive");
    let label = std::env::var("MARKET_BENCH_LABEL").unwrap_or_else(|_| "local".into());
    let rows = fixture(count);
    println!(
        "{}",
        serde_json::json!({"label":label,"metric":"fixture","orders":count,"venues":8,"collections":2,"currencies":2,"models":16,"page_size":200,"profile_allocations":cfg!(feature="profile-allocations")})
    );
    #[cfg(feature = "profile-allocations")]
    let region = stats_alloc::Region::new(ALLOCATOR);
    let mut state = State::default();
    state.apply(&rows).unwrap();
    #[cfg(feature = "profile-allocations")]
    {
        let stats = region.change();
        println!(
            "{}",
            serde_json::json!({"label":label,"case":"retained_state","metric":"memory","allocated_bytes":stats.bytes_allocated,"freed_bytes":stats.bytes_deallocated,"reallocated_bytes":stats.bytes_reallocated})
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "ranked-benchmark").unwrap();
    let mut snapshot = writer.begin_snapshot().unwrap();
    snapshot.append(&rows).unwrap();
    writer
        .install_snapshot(snapshot.finish("bench").unwrap())
        .unwrap();
    let reader = writer.reader();
    for (name, sort, venue, sparse) in [
        ("price_multi", Sort::PriceAsc, None, false),
        ("price_single", Sort::PriceAsc, Some("v0"), false),
        ("time_desc_multi", Sort::TimeDesc, None, false),
        ("number_desc_multi", Sort::NumberDesc, None, false),
        ("price_sparse", Sort::PriceAsc, None, true),
        ("identity_control", Sort::Identity, None, false),
    ] {
        let request = query(w::ReadSort::Identity, venue, sparse);
        let options = ReadOptions {
            sort: Some(sort),
            ..Default::default()
        };
        assert_eq!(
            reader
                .read_with_options(request.clone(), options)
                .unwrap()
                .page
                .entities
                .len(),
            200,
            "{name}: fixture must fill a page"
        );
        measure(&label, name, iterations, || {
            black_box(
                reader
                    .read_with_options(black_box(request.clone()), options)
                    .unwrap(),
            );
        });
    }
    let explicit_keys = rows
        .iter()
        .filter_map(|row| match &row.change {
            Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::Order(order)),
            })) => Some(w::Key {
                kind: w::EntityKind::Order as i32,
                venue_id: order.book.as_ref().unwrap().venue_id.clone(),
                id: order.id.clone(),
            }),
            _ => None,
        })
        .take(200)
        .collect();
    for (name, selection, expected) in [
        (
            "identity_orders",
            w::Selection {
                kinds: vec![w::EntityKind::Order as i32],
                ..Default::default()
            },
            200,
        ),
        (
            "identity_keys",
            w::Selection {
                keys: explicit_keys,
                ..Default::default()
            },
            200,
        ),
        ("identity_mixed", w::Selection::default(), 200),
        (
            "metadata_options",
            w::Selection {
                kinds: vec![w::EntityKind::AttributeOption as i32],
                ..Default::default()
            },
            16,
        ),
    ] {
        let request = w::ReadRequest {
            selection: Some(selection),
            sort: 0,
            mode: 1,
            limit: 200,
            ..Default::default()
        };
        assert_eq!(
            reader.read(request.clone()).unwrap().page.entities.len(),
            expected,
            "{name}"
        );
        measure(&label, name, iterations, || {
            black_box(reader.read(black_box(request.clone())).unwrap());
        });
    }
    let mut continuation = query(w::ReadSort::PriceAsc, None, false);
    for _ in 0..5 {
        let page = reader.read(continuation.clone()).unwrap().page;
        assert!(!page.next_page_token.is_empty());
        continuation.page_token = page.next_page_token;
    }
    measure(&label, "price_continuation", iterations, || {
        black_box(reader.read(black_box(continuation.clone())).unwrap());
    });
    let originals: Vec<_> = rows
        .iter()
        .filter(|row| {
            matches!(
                &row.change,
                Some(w::change::Change::Upsert(w::Entity {
                    entity: Some(w::entity::Entity::Order(_))
                }))
            )
        })
        .take(100)
        .cloned()
        .collect();
    let mut repriced = originals.clone();
    let mut moved = originals.clone();
    let mut metadata = originals.clone();
    let mut removals = Vec::new();
    for ((reprice, movement), meta) in repriced.iter_mut().zip(&mut moved).zip(&mut metadata) {
        order_mut(reprice).price = "20000.000000009".into();
        let order = order_mut(movement);
        order.mode = if order.mode == 1 { 2 } else { 1 };
        order.book.as_mut().unwrap().item_type_id = "gift.moved".into();
        order_mut(meta)
            .meta
            .insert("changed".into(), "new payload, same rank".into());
    }
    for original in &originals {
        if let Some(w::change::Change::Upsert(w::Entity {
            entity: Some(w::entity::Entity::Order(order)),
        })) = &original.change
        {
            removals.push(w::Change {
                change: Some(w::change::Change::Remove(w::Removal {
                    key: Some(w::Key {
                        kind: w::EntityKind::Order as i32,
                        venue_id: order.book.as_ref().unwrap().venue_id.clone(),
                        id: order.id.clone(),
                    }),
                    ..Default::default()
                })),
            });
        }
    }
    for (name, changes) in [
        ("update_100_price", repriced),
        ("update_100_metadata", metadata),
        ("update_100_partition", moved),
        ("remove_100", removals),
    ] {
        measure(&label, name, iterations, || {
            let old = state.clone();
            let mut next = old.clone();
            next.apply(black_box(&changes)).unwrap();
            black_box(&next);
        });
    }
    let updates = originals;
    let mut sequence = 0;
    measure(&label, "persist_update_100", iterations, || {
        sequence += 1;
        writer
            .apply(w::Batch {
                cursor: sequence.to_string(),
                changes: updates.clone(),
            })
            .unwrap();
    });
}
