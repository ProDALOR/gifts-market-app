use super::*;
use crate::test_support::{catalog, entity, order, read, upsert};
use crate::types::{Key, Sort};
use prost::Message;
use rust_decimal::Decimal;
use w::entity::Entity as E;
mod sparse;
fn setup() -> (tempfile::TempDir, ReplicaWriter) {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(dir.path(), "test").unwrap();
    let mut changes = catalog();
    for i in 1..=20 {
        changes.push(upsert(entity(E::Order(order(
            &format!("{i:02}"),
            &i.to_string(),
            None,
        )))));
    }
    let mut builder = writer.begin_snapshot().unwrap();
    builder.append(&changes).unwrap();
    writer
        .install_snapshot(builder.finish("first").unwrap())
        .unwrap();
    (dir, writer)
}
fn ids(window: &ReadBatch) -> Vec<String> {
    window
        .entries
        .iter()
        .map(|e| state::order(&e.entity).unwrap().id.clone())
        .collect()
}
#[test]
fn public_ordering_keys_preserve_exact_rank_identity_prefixes_and_direction() {
    let key = Key::order("v".into(), "a".into());
    assert_eq!(
        super::range::ordering_key(Decimal::ZERO, &key, Sort::PriceAsc),
        super::range::ordering_key(Decimal::from_str_exact("-0").unwrap(), &key, Sort::PriceAsc)
    );
    let ranks = [
        Decimal::from_str_exact("10000000000000000.000000001").unwrap(),
        Decimal::from_str_exact("10000000000000000.000000002").unwrap(),
    ];
    let keys = ["a", "a\0", "aa", "z", "💎"].map(|id| Key::order("v".into(), id.into()));
    for sort in [Sort::PriceAsc, Sort::PriceDesc] {
        let mut ordered = ranks
            .into_iter()
            .flat_map(|rank| keys.iter().cloned().map(move |key| (rank, key)))
            .collect::<Vec<_>>();
        if sort.descending() {
            ordered.reverse();
        }
        let encoded = ordered
            .into_iter()
            .map(|(rank, key)| super::range::ordering_key(rank, &key, sort))
            .collect::<Vec<_>>();
        assert!(encoded.windows(2).all(|pair| pair[0] < pair[1]));
    }
}

#[test]
fn range_queries_page_without_duplicates_and_watches_reject_incomplete_coverage() {
    let (_dir, writer) = setup();
    let reader = writer.reader();
    for sort in 0..=6 {
        let request = read(1, false);
        let options = ReadOptions {
            sort: Some(Sort::try_from(sort).unwrap()),
            ..Default::default()
        };
        let bounds = KeyRange {
            start: None,
            end: None,
        };
        let mut start = None;
        let mut seen = Vec::new();
        loop {
            let page = reader
                .query_range_checked(
                    request.clone(),
                    RangeRequest {
                        options,
                        range: KeyRange {
                            start: start.clone(),
                            end: None,
                        },
                        limit: 5,
                        snapshot: None,
                    },
                    false,
                    |_, _| Ok(()),
                    |_| Ok(true),
                )
                .unwrap();
            assert_eq!(page.range.start, start);
            assert!(page.entries.len() <= 5);
            seen.extend(ids(&page));
            let Some(next) = page.range.end else {
                break;
            };
            assert!(start.as_ref().is_none_or(|old| old < &next));
            start = Some(next);
        }
        assert_eq!(seen.len(), 20);
        assert_eq!(
            seen.iter().collect::<std::collections::BTreeSet<_>>().len(),
            20
        );
        let complete = reader
            .query_range_checked(
                request.clone(),
                RangeRequest {
                    options,
                    range: bounds.clone(),
                    limit: 20,
                    snapshot: None,
                },
                true,
                |_, _| Ok(()),
                |_| Ok(true),
            )
            .unwrap();
        assert_eq!(seen, ids(&complete));
        assert_eq!(
            reader
                .query_range_checked(
                    request,
                    RangeRequest {
                        options,
                        range: bounds,
                        limit: 19,
                        snapshot: None
                    },
                    true,
                    |_, _| Ok(()),
                    |_| Ok(true)
                )
                .err()
                .unwrap()
                .code(),
            tonic::Code::ResourceExhausted
        );
    }
}

#[test]
fn half_open_bounded_queries_advance_to_the_original_end_for_every_source_and_sort() {
    let (_dir, mut writer) = setup();
    writer
        .commit(
            (1..=12)
                .map(|i| {
                    upsert(entity(E::Order(order(
                        &format!("h{i:02}"),
                        &i.to_string(),
                        Some(i * 100),
                    ))))
                })
                .collect(),
            "history-bounds".into(),
        )
        .unwrap();
    let reader = writer.reader();
    for history in [false, true] {
        for sort in 0..=6 {
            let request = read(1, history);
            let options = ReadOptions {
                sort: Some(Sort::try_from(sort).unwrap()),
                ..Default::default()
            };
            let full = reader
                .query_range_checked(
                    request.clone(),
                    RangeRequest {
                        options,
                        range: KeyRange {
                            start: None,
                            end: None,
                        },
                        limit: 100,
                        snapshot: None,
                    },
                    false,
                    |_, _| Ok(()),
                    |_| Ok(true),
                )
                .unwrap();
            let end = Some(full.entries[8].key.clone());
            let mut start = Some(full.entries[2].key.clone());
            let mut seen = vec![];
            loop {
                let page = reader
                    .query_range_checked(
                        request.clone(),
                        RangeRequest {
                            options,
                            range: KeyRange {
                                start: start.clone(),
                                end: end.clone(),
                            },
                            limit: 2,
                            snapshot: None,
                        },
                        false,
                        |_, _| Ok(()),
                        |_| Ok(true),
                    )
                    .unwrap();
                seen.extend(ids(&page));
                if page.range.end == end {
                    break;
                }
                assert!(page.range.end > start);
                start = page.range.end;
            }
            assert_eq!(seen, ids(&full)[2..8]);
            let empty = reader
                .query_range_checked(
                    request.clone(),
                    RangeRequest {
                        options,
                        range: KeyRange {
                            start: end.clone(),
                            end: end.clone(),
                        },
                        limit: 2,
                        snapshot: None,
                    },
                    false,
                    |_, _| Ok(()),
                    |_| Ok(true),
                )
                .unwrap();
            assert!(empty.entries.is_empty());
            assert_eq!(empty.range.start, end);
            assert_eq!(empty.range.end, end);
        }
    }
}

#[test]
fn read_observation_clocks_belong_to_the_pinned_view_including_retained_snapshots() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let mut request = read(1, false);
    request.mode = w::ReadMode::Snapshot as i32;
    let bounds = KeyRange {
        start: None,
        end: None,
    };
    let first = reader
        .query_range_checked(
            request.clone(),
            RangeRequest {
                options: Default::default(),
                range: bounds.clone(),
                limit: 20,
                snapshot: None,
            },
            false,
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    writer
        .commit(
            vec![upsert(entity(E::Order(order("01", "0.5", None))))],
            "new-observation".into(),
        )
        .unwrap();
    let latest = reader
        .query_range_checked(
            read(1, false),
            RangeRequest {
                options: Default::default(),
                range: bounds.clone(),
                limit: 20,
                snapshot: None,
            },
            false,
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    let retained = reader
        .query_range_checked(
            request,
            RangeRequest {
                options: Default::default(),
                range: bounds,
                limit: 20,
                snapshot: first.snapshot,
            },
            false,
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert!(latest.observed > first.observed);
    assert_eq!(retained.observed, first.observed);
    assert_eq!(retained.revision, first.revision);
    assert_ne!(latest.revision, first.revision);
}

fn page(
    reader: &ReplicaReader,
    request: w::ReadRequest,
    range: KeyRange,
    sort: Sort,
    limit: u32,
    snapshot: Option<SnapshotId>,
) -> Result<ReadBatch, Status> {
    reader.query_range_checked(
        request,
        RangeRequest {
            range,
            limit,
            snapshot,
            options: ReadOptions {
                sort: Some(sort),
                ..Default::default()
            },
        },
        false,
        |_, _| Ok(()),
        |_| Ok(true),
    )
}
fn open() -> KeyRange {
    KeyRange {
        start: None,
        end: None,
    }
}

#[test]
fn fixed_bounds_survive_repricing_deletion_and_insertion_without_following_an_anchor() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let all = page(&reader, read(1, false), open(), Sort::PriceAsc, 100, None).unwrap();
    let range = KeyRange {
        start: Some(all.entries[9].key.clone()),
        end: Some(all.entries[14].key.clone()),
    };
    writer
        .commit(
            vec![
                upsert(entity(E::Order(order("10", "100", None)))),
                upsert(entity(E::Order(order("new", "10.5", None)))),
            ],
            "repriced".into(),
        )
        .unwrap();
    let current = page(
        &reader,
        read(1, false),
        range.clone(),
        Sort::PriceAsc,
        100,
        None,
    )
    .unwrap();
    assert_eq!(ids(&current), ["new", "11", "12", "13", "14"]);
    assert_eq!(current.range, range);
    assert_eq!(
        page(&reader, read(1, false), range, Sort::PriceDesc, 100, None)
            .err()
            .unwrap()
            .code(),
        tonic::Code::InvalidArgument
    );
}

#[test]
fn pinned_ranges_remain_frozen_and_released_leases_cannot_reappear() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let first = page(&reader, read(0, false), open(), Sort::PriceAsc, 5, None).unwrap();
    let lease = first.snapshot.unwrap();
    writer
        .commit(
            vec![upsert(entity(E::Order(order("06", "0.5", None))))],
            "updated".into(),
        )
        .unwrap();
    let bounds = KeyRange {
        start: first.range.end.clone(),
        end: None,
    };
    let frozen = page(
        &reader,
        read(0, false),
        bounds.clone(),
        Sort::PriceAsc,
        5,
        Some(lease),
    )
    .unwrap();
    assert_eq!(ids(&frozen), ["06", "07", "08", "09", "10"]);
    let latest = page(
        &reader,
        read(1, false),
        bounds.clone(),
        Sort::PriceAsc,
        5,
        None,
    )
    .unwrap();
    assert_eq!(ids(&latest), ["07", "08", "09", "10", "11"]);
    reader.release_snapshot(lease).unwrap();
    assert!(
        page(
            &reader,
            read(0, false),
            bounds.clone(),
            Sort::PriceAsc,
            5,
            Some(lease)
        )
        .is_err()
    );
    let new = page(
        &reader,
        read(0, false),
        bounds.clone(),
        Sort::PriceAsc,
        5,
        None,
    )
    .unwrap();
    let id = new.snapshot.unwrap();
    let failed = reader.query_range_checked(
        read(0, false),
        RangeRequest {
            range: bounds,
            limit: 5,
            snapshot: Some(id),
            options: ReadOptions::default(),
        },
        false,
        |_, _| {
            reader.release_snapshot(id)?;
            Ok(())
        },
        |_| Ok(true),
    );
    assert_eq!(
        failed.err().unwrap().code(),
        tonic::Code::FailedPrecondition
    );
    assert!(reader.0.views.lock().unwrap().is_empty());
}

#[test]
fn ranges_enforce_limits_cancellation_source_and_half_open_empty_bounds() {
    let (_dir, writer) = setup();
    let reader = writer.reader();
    for limit in [0, READ_LIMITS.max_batch_rows + 1] {
        assert_eq!(
            page(&reader, read(1, false), open(), Sort::PriceAsc, limit, None)
                .err()
                .unwrap()
                .code(),
            tonic::Code::InvalidArgument
        );
    }
    let all = page(&reader, read(1, false), open(), Sort::PriceAsc, 100, None).unwrap();
    let inverse = KeyRange {
        start: Some(all.entries[5].key.clone()),
        end: Some(all.entries[2].key.clone()),
    };
    assert!(page(&reader, read(1, false), inverse, Sort::PriceAsc, 100, None).is_err());
    let mut bids = read(1, false);
    if let Some(w::selection::Source::Scope(scope)) = &mut bids.selection.as_mut().unwrap().source {
        scope.modes = vec![2];
    }
    assert!(
        page(
            &reader,
            bids,
            KeyRange {
                start: Some(all.entries[0].key.clone()),
                end: None
            },
            Sort::PriceAsc,
            100,
            None
        )
        .is_err()
    );
    let stop = tokio_util::sync::CancellationToken::new();
    let cancelled = reader.with_cancellation(stop.clone());
    stop.cancel();
    assert!(
        page(
            &cancelled,
            read(1, false),
            open(),
            Sort::PriceAsc,
            100,
            None
        )
        .is_err()
    );
}

#[test]
fn row_policy_is_applied_before_limit_and_range_publication_is_one_pinned_read() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let batch = reader
        .query_range_checked(
            read(1, false),
            RangeRequest {
                range: open(),
                limit: 3,
                snapshot: None,
                options: ReadOptions::default(),
            },
            false,
            |_, _| {
                writer.commit(
                    vec![upsert(entity(E::Order(order("01", "100", None))))],
                    "during-read".into(),
                )?;
                Ok(())
            },
            |entity| Ok(state::order(entity).unwrap().id.parse::<usize>().unwrap() % 2 == 0),
        )
        .unwrap();
    assert_eq!(ids(&batch), ["02", "04", "06"]);
    let latest = page(&reader, read(1, false), open(), Sort::PriceAsc, 3, None).unwrap();
    assert!(latest.observed > batch.observed);
    assert_ne!(latest.revision, batch.revision);
}

trait Commit {
    fn commit(&mut self, changes: Vec<w::Change>, cursor: String) -> Result<(), Status>;
}
impl Commit for ReplicaWriter {
    fn commit(&mut self, changes: Vec<w::Change>, cursor: String) -> Result<(), Status> {
        self.apply(w::Batch { changes, cursor })
    }
}

#[test]
fn local_ranking_cannot_reintroduce_undefined_wire_sort_codes() {
    let (_directory, writer) = setup();
    let reader = writer.reader();
    let mut wire = read(1, false);
    wire.sort = 3;
    assert!(w::ReadSort::try_from(wire.sort).is_err());
    assert!(reader.read(wire.clone()).is_err());
    assert!(page(&reader, wire, open(), Sort::TimeAsc, 5, None).is_err());
    let valid = page(&reader, read(1, false), open(), Sort::TimeAsc, 5, None).unwrap();
    assert_eq!(valid.entries.len(), 5);
}

#[test]
fn sparse_row_policy_advances_empty_pages_without_claiming_unscanned_coverage() {
    let (directory, _) = setup();
    let mut writer =
        ReplicaWriter::open(&directory.path().join("sparse-policy"), "sparse-policy").unwrap();
    let mut builder = writer.begin_snapshot().unwrap();
    builder.append(&catalog()).unwrap();
    for history in [false, true] {
        let changes = (0..READ_LIMITS.max_scan_candidates + 2)
            .map(|i| {
                let mut row = order(
                    &format!("{}-{i:06}", if history { "h" } else { "a" }),
                    "1",
                    history.then_some(i as i64 + 1),
                );
                row.observed_at_ms = i as i64 + 1;
                upsert(entity(E::Order(row)))
            })
            .collect::<Vec<_>>();
        builder.append(&changes).unwrap();
    }
    writer
        .install_snapshot(builder.finish("sparse-policy").unwrap())
        .unwrap();
    let reader = writer.reader();
    for history in [false, true] {
        let include = |entity: &w::Entity| {
            let id = &state::order(entity).unwrap().id;
            let index = id.split_once('-').unwrap().1.parse::<usize>().unwrap();
            Ok(index >= READ_LIMITS.max_scan_candidates)
        };
        let spec = |range| RangeRequest {
            range,
            limit: 10,
            snapshot: None,
            options: ReadOptions {
                sort: Some(Sort::TimeAsc),
                ..Default::default()
            },
        };
        let first = reader
            .query_range_checked(
                read(1, history),
                spec(open()),
                false,
                |_, _| Ok(()),
                include,
            )
            .unwrap();
        assert!(first.entries.is_empty());
        assert!(
            first.range.end.is_some(),
            "an empty budget page must advance"
        );
        let second = reader
            .query_range_checked(
                read(1, history),
                spec(KeyRange {
                    start: first.range.end.clone(),
                    end: None,
                }),
                false,
                |_, _| Ok(()),
                include,
            )
            .unwrap();
        assert_eq!(second.entries.len(), 2);
        assert!(second.range.end.is_none());
        assert!(
            reader
                .query_range_checked(read(1, history), spec(open()), true, |_, _| Ok(()), include)
                .is_err(),
            "a complete watch must not advertise partial coverage"
        );
    }
}

#[test]
fn ranges_enforce_byte_budgets_after_row_policy_without_retaining_failed_snapshots() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "range-bytes").unwrap();
    let mut builder = writer.begin_snapshot().unwrap();
    builder.append(&catalog()).unwrap();
    for history in [false, true] {
        let rows = (0..3)
            .map(|i| {
                let mut row = order(
                    &format!("{}-{i}", if history { "h" } else { "a" }),
                    "1",
                    history.then_some(i + 1),
                );
                if i < 2 {
                    row.meta.insert("large".into(), "x".repeat(300 * 1024));
                }
                upsert(entity(E::Order(row)))
            })
            .collect::<Vec<_>>();
        builder.append(&rows).unwrap();
    }
    writer
        .install_snapshot(builder.finish("range-bytes").unwrap())
        .unwrap();
    let reader = writer.reader();
    for history in [false, true] {
        let spec = || RangeRequest {
            range: open(),
            limit: 3,
            snapshot: None,
            options: ReadOptions::default(),
        };
        for complete in [false, true] {
            let result = reader.query_range_checked(
                read(0, history),
                spec(),
                complete,
                |_, _| Ok(()),
                |_| Ok(true),
            );
            match result {
                Err(error) => {
                    assert_eq!(error.code(), tonic::Code::ResourceExhausted);
                    assert!(reader.0.views.lock().unwrap().is_empty());
                }
                Ok(page) => {
                    // Finite archive reads can stop at their storage byte budget;
                    // complete watches must reject that incomplete coverage.
                    assert!(history && !complete);
                    assert_eq!(page.entries.len(), 1);
                    assert!(page.range.end.is_some());
                    assert!(
                        page.entries
                            .iter()
                            .map(|row| row.entity.encoded_len())
                            .sum::<usize>()
                            <= READ_LIMITS.max_read_bytes
                    );
                    reader.release_snapshot(page.snapshot.unwrap()).unwrap();
                    assert!(reader.0.views.lock().unwrap().is_empty());
                }
            }
        }
        let small = reader
            .query_range_checked(
                read(1, history),
                spec(),
                false,
                |_, _| Ok(()),
                |entity| Ok(!state::order(entity).unwrap().meta.contains_key("large")),
            )
            .unwrap();
        assert_eq!(small.entries.len(), 1);
        assert!(small.range.end.is_none());
    }
}
