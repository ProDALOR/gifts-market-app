use super::*;
use crate::test_support::{catalog, entity, order, read, upsert};
use crate::types::{Key, Sort};
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
fn window(
    reader: &ReplicaReader,
    sort: i32,
    anchor: Option<ReadPosition>,
    before: u32,
    after: u32,
) -> ReadBatch {
    let mut request = read(1, false);
    request.sort = sort;
    reader
        .read_slice_checked(
            request,
            seed(anchor, before, after, None),
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap()
}
fn ids(window: &ReadBatch) -> Vec<String> {
    window
        .entries
        .iter()
        .map(|e| state::order(&e.entity).unwrap().id.clone())
        .collect()
}
#[test]
fn window_navigation_is_contiguous_in_both_directions_and_fills_at_edges() {
    let (_dir, writer) = setup();
    let reader = writer.reader();
    for sort in 0..=6 {
        let first = window(&reader, sort, None, 2, 2);
        assert_eq!(first.entries.len(), 5);
        assert!(
            first
                .entries
                .windows(2)
                .all(|pair| pair[0].key < pair[1].key)
        );
        assert!(!first.range.start.is_some());
        assert!(first.range.end.is_some());
        let middle = window(&reader, sort, Some(first.entries[4].key.clone()), 2, 2);
        assert_eq!(middle.entries[1].key, first.entries[4].key);
        assert!(middle.range.start.is_some());
        assert!(middle.range.end.is_some());
        let back = window(&reader, sort, Some(first.entries[1].key.clone()), 2, 2);
        assert_eq!(ids(&back), ids(&first));
        let mut last = middle;
        while last.range.end.is_some() {
            last = window(&reader, sort, Some(last.entries[4].key.clone()), 2, 2);
        }
        assert_eq!(last.entries.len(), 5);
        assert!(last.range.start.is_some());
        assert!(!last.range.end.is_some());
    }
}
#[test]
fn inserts_removals_and_price_moves_preserve_identity_and_nearest_fallback() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let initial = window(&reader, 1, None, 2, 2);
    let anchor = initial.entries[4].key.clone();
    let old = window(&reader, 1, Some(anchor.clone()), 2, 2);
    assert_eq!(ids(&old), ["04", "05", "06", "07", "08"]);
    writer
        .commit(
            vec![upsert(entity(E::Order(order("new", "4.5", None))))],
            "insert".into(),
        )
        .unwrap();
    let next = window(&reader, 1, Some(anchor.clone()), 2, 2);
    assert_eq!(ids(&next), ["new", "05", "06", "07", "08"]);
    writer
        .commit(
            vec![upsert(entity(E::Order(order("05", "15.5", None))))],
            "move".into(),
        )
        .unwrap();
    let next = window(&reader, 1, Some(anchor), 2, 2);
    assert_eq!(ids(&next), ["15", "05", "16", "17", "18"]);
    let moved = next.entries[1].key.clone();
    writer
        .commit(
            vec![w::Change {
                change: Some(w::change::Change::Remove(w::Removal {
                    key: Some(w::Key {
                        kind: 7,
                        venue_id: "venue".into(),
                        id: "05".into(),
                    }),
                    ..Default::default()
                })),
            }],
            "remove".into(),
        )
        .unwrap();
    let next = window(&reader, 1, Some(moved), 2, 2);
    assert_eq!(ids(&next), ["15", "16", "17", "18", "19"]);
}
#[test]
fn a_window_and_its_revision_are_pinned_even_when_a_commit_occurs_during_scan() {
    let (_dir, writer) = setup();
    let reader = writer.reader();
    let writer = std::sync::Mutex::new(Some(writer));
    let expected = window(&reader, 1, None, 2, 2);
    let mut request = read(1, false);
    request.sort = 1;
    let actual = reader
        .read_slice_checked(
            request,
            seed(None, 2, 2, None),
            |_, _| Ok(()),
            |_| {
                if let Some(mut writer) = writer.lock().unwrap().take() {
                    writer
                        .commit(
                            vec![upsert(entity(E::Order(order("new", "0.1", None))))],
                            "new".into(),
                        )
                        .unwrap();
                }
                Ok(true)
            },
        )
        .unwrap();
    assert_eq!(ids(&actual), ids(&expected));
    assert_eq!(actual.revision, expected.revision);
    assert_ne!(ids(&window(&reader, 1, None, 2, 2)), ids(&expected));
}
#[test]
fn anchors_validate_sort_and_source_and_enforce_limits_and_cancellation() {
    let (_dir, writer) = setup();
    let reader = writer.reader();
    let first = window(&reader, 1, None, 2, 2);
    let mut request = read(1, false);
    request.sort = 2;
    assert!(
        reader
            .read_slice_checked(
                request,
                seed(Some(first.entries[0].key.clone()), 2, 2, None),
                |_, _| Ok(()),
                |_| Ok(true)
            )
            .is_err()
    );
    for (before, after) in [(200, 0), (u32::MAX, 1)] {
        assert!(
            reader
                .read_slice_checked(
                    read(1, false),
                    seed(None, before, after, None),
                    |_, _| Ok(()),
                    |_| Ok(true)
                )
                .is_err()
        );
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    assert!(
        reader
            .with_cancellation(cancel)
            .read_slice_checked(
                read(1, false),
                seed(None, 1, 1, None),
                |_, _| Ok(()),
                |_| Ok(true)
            )
            .is_err()
    );
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
fn bid_windows_share_ranking_and_apply_row_policy_before_accounting() {
    let (_dir, mut writer) = setup();
    let changes = (1..=20)
        .map(|i| {
            let mut value = order(&format!("{i:02}"), &i.to_string(), None);
            value.mode = 2;
            if i == 20 {
                value.quantity = "0".into();
            }
            upsert(entity(E::Order(value)))
        })
        .collect();
    writer
        .apply(w::Batch {
            changes,
            cursor: "bids".into(),
        })
        .unwrap();
    let mut request = read(1, false);
    request.sort = 2;
    let Some(w::selection::Source::Scope(scope)) = &mut request.selection.as_mut().unwrap().source
    else {
        unreachable!()
    };
    scope.modes = vec![2];
    let result = writer
        .reader()
        .read_slice_checked(
            request,
            seed(None, 2, 2, None),
            |_, _| Ok(()),
            |e| Ok(state::order(e).unwrap().quantity != "0"),
        )
        .unwrap();
    assert_eq!(ids(&result), ["19", "18", "17", "16", "15"]);
    assert!(result.range.end.is_some());
}
#[test]
fn empty_history_seeds_and_oversized_reads_do_not_return_partial_results() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    assert!(
        reader
            .read_slice_checked(
                read(1, true),
                seed(None, 1, 1, None),
                |_, _| Ok(()),
                |_| Ok(true)
            )
            .unwrap()
            .entries
            .is_empty()
    );
    let changes = ["01", "02"]
        .into_iter()
        .map(|id| {
            let mut row = order(id, "1", None);
            row.meta.insert("large".into(), "x".repeat(300 * 1024));
            upsert(entity(E::Order(row)))
        })
        .collect();
    writer
        .apply(w::Batch {
            changes,
            cursor: "large".into(),
        })
        .unwrap();
    let error = reader
        .read_slice_checked(
            read(1, false),
            seed(None, 1, 1, None),
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .err()
        .unwrap();
    assert_eq!(error.code(), tonic::Code::ResourceExhausted);
    assert!(reader.status().ready);
}

#[test]
fn frozen_windows_open_at_a_live_anchor_stay_frozen_and_release_their_view() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let live = window(&reader, 1, None, 0, 19);
    let anchor = live.entries[15].key.clone();
    let frozen = reader
        .read_slice_checked(
            read(0, false),
            seed(Some(anchor.clone()), 2, 2, None),
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&frozen), ["15", "16", "17", "18", "19"]);
    let lease = frozen.snapshot.unwrap();
    writer
        .apply(w::Batch {
            cursor: "moved".into(),
            changes: vec![upsert(entity(E::Order(order("16", "0.1", None))))],
        })
        .unwrap();
    let same = reader
        .read_slice_checked(
            read(0, false),
            seed(Some(anchor.clone()), 2, 2, Some(lease)),
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&same), ids(&frozen));
    assert_eq!(same.revision, frozen.revision);
    assert_ne!(ids(&window(&reader, 1, Some(anchor), 2, 2)), ids(&frozen));
    reader.release_snapshot(lease).unwrap();
    reader.release_snapshot(lease).unwrap();
    assert!(
        !reader
            .0
            .views
            .lock()
            .unwrap()
            .contains_key(&lease.to_string())
    );
    let error = reader
        .read_slice_checked(
            read(0, false),
            seed(Some(same.entries[2].key.clone()), 2, 2, Some(lease)),
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .err()
        .unwrap();
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
}

#[test]
fn releasing_a_snapshot_wins_over_an_inflight_window_read() {
    let (_dir, writer) = setup();
    let reader = writer.reader();
    let first = reader
        .read_slice_checked(
            read(0, false),
            seed(None, 2, 2, None),
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    let lease = first.snapshot.unwrap();
    let result = reader.read_slice_checked(
        read(0, false),
        seed(Some(first.entries[0].key.clone()), 2, 2, Some(lease)),
        |_, _| Ok(()),
        |_| {
            reader.release_snapshot(lease)?;
            Ok(true)
        },
    );
    assert!(result.is_err());
    assert!(
        !reader
            .0
            .views
            .lock()
            .unwrap()
            .contains_key(&lease.to_string())
    );
}

#[test]
fn public_ordering_keys_preserve_exact_rank_identity_prefixes_and_direction() {
    let key = Key::order("v".into(), "a".into());
    assert_eq!(
        super::slice::ordering_key(Decimal::ZERO, &key, Sort::PriceAsc),
        super::slice::ordering_key(Decimal::from_str_exact("-0").unwrap(), &key, Sort::PriceAsc)
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
            .map(|(rank, key)| super::slice::ordering_key(rank, &key, sort))
            .collect::<Vec<_>>();
        assert!(encoded.windows(2).all(|pair| pair[0] < pair[1]));
    }
}

#[test]
fn latest_windows_do_not_retain_views_and_reject_snapshot_leases() {
    let (_dir, writer) = setup();
    let reader = writer.reader();
    let latest = window(&reader, 1, None, 0, READ_LIMITS.max_rows - 1);
    assert!(latest.snapshot.is_none());
    assert!(reader.0.views.lock().unwrap().is_empty());
    let error = reader
        .read_slice_checked(
            read(1, false),
            seed(None, 0, 0, Some(SnapshotId::fresh())),
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .err()
        .unwrap();
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert!(reader.0.views.lock().unwrap().is_empty());
}
#[test]
fn anchor_falls_back_to_previous_at_end_and_reports_empty_when_no_rows_remain() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let all = window(&reader, 1, None, 0, 19);
    let anchor = all.entries.last().unwrap().key.clone();
    let remove = |id: &str| w::Change {
        change: Some(w::change::Change::Remove(w::Removal {
            key: Some(w::Key {
                kind: 7,
                venue_id: "venue".into(),
                id: id.into(),
            }),
            ..Default::default()
        })),
    };
    writer
        .commit(vec![remove("20")], "last-removed".into())
        .unwrap();
    let previous = window(&reader, 1, Some(anchor.clone()), 2, 2);
    assert_eq!(ids(&previous), ["15", "16", "17", "18", "19"]);
    writer
        .commit(
            (1..20).map(|i| remove(&format!("{i:02}"))).collect(),
            "empty".into(),
        )
        .unwrap();
    let empty = window(&reader, 1, Some(anchor), 2, 2);
    assert!(empty.entries.is_empty());
    assert!(!empty.range.start.is_some() && !empty.range.end.is_some());
}

#[test]
fn fixed_ranges_use_original_ordering_bounds_after_anchor_repricing() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let all = window(&reader, 1, None, 0, 19);
    let coverage = KeyRange {
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
    let current = reader
        .read_range_checked(
            read(1, false),
            coverage.clone(),
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&current), ["new", "11", "12", "13", "14"]);
    let mut foreign = read(1, false);
    foreign.sort = 2;
    assert_eq!(
        reader
            .read_range_checked(foreign, coverage, |_, _| Ok(()), |_| Ok(true))
            .err()
            .unwrap()
            .code(),
        tonic::Code::InvalidArgument
    );
}
#[test]
fn history_slices_follow_all_ranked_indexes_and_include_late_backfills_in_coverage() {
    let (_dir, mut writer) = setup();
    writer
        .commit(
            vec![
                upsert(entity(E::Order(order("h1", "10", Some(100))))),
                upsert(entity(E::Order(order("h2", "10", Some(200))))),
                upsert(entity(E::Order(order("h3", "11", Some(150))))),
                upsert(entity(E::Order(order("h4", "8", Some(150))))),
            ],
            "history".into(),
        )
        .unwrap();
    let reader = writer.reader();
    for sort in 0..=6 {
        let mut query = read(1, true);
        query.sort = sort;
        let slice = reader
            .read_slice_checked(
                query.clone(),
                seed(None, 1, 1, None),
                |_, _| Ok(()),
                |_| Ok(true),
            )
            .unwrap();
        assert_eq!(slice.entries.len(), 3);
        assert!(
            slice
                .entries
                .windows(2)
                .all(|pair| pair[0].key < pair[1].key)
        );
        let singleton = reader
            .read_range_checked(
                query.clone(),
                KeyRange {
                    start: Some(slice.entries[1].key.clone()),
                    end: Some(slice.entries[1].key.after()),
                },
                |_, _| Ok(()),
                |_| Ok(true),
            )
            .unwrap();
        assert_eq!(ids(&singleton), vec![ids(&slice)[1].clone()]);
        let middle = reader
            .read_slice_checked(
                query,
                seed(Some(slice.entries[1].key.clone()), 1, 1, None),
                |_, _| Ok(()),
                |_| Ok(true),
            )
            .unwrap();
        assert_eq!(ids(&middle), ids(&slice));
    }
    let mut query = read(1, true);
    query.sort = w::ReadSort::TimeDesc as i32;
    let slice = reader
        .read_slice_checked(
            query.clone(),
            seed(None, 0, 1, None),
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&slice), ["h2", "h4"]);
    let coverage = KeyRange {
        start: None,
        end: Some(slice.entries[1].key.after()),
    };
    writer
        .commit(
            vec![
                upsert(entity(E::Order(order("late", "1", Some(175))))),
                upsert(entity(E::Order(order("older", "1", Some(50))))),
                upsert(entity(E::Order(order("latest", "1", Some(300))))),
            ],
            "backfill".into(),
        )
        .unwrap();
    let current = reader
        .read_range_checked(query, coverage, |_, _| Ok(()), |_| Ok(true))
        .unwrap();
    assert_eq!(ids(&current), ["latest", "h2", "late", "h4"]);
    assert!(
        current
            .entries
            .windows(2)
            .all(|pair| pair[0].key < pair[1].key)
    );
}

#[test]
fn active_price_changes_do_not_invalidate_history_reads_but_backfills_do() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let query = read(1, true);
    let before = reader.read_revision(&query).unwrap();
    writer
        .commit(
            vec![upsert(entity(E::Order(order("01", "0.1", None))))],
            "price".into(),
        )
        .unwrap();
    assert_eq!(before, reader.read_revision(&query).unwrap());
    writer
        .commit(
            vec![upsert(entity(E::Order(order("late", "1", Some(100)))))],
            "late".into(),
        )
        .unwrap();
    assert_ne!(before, reader.read_revision(&query).unwrap());
}

#[test]
fn directional_slices_do_not_fill_from_the_opposite_side() {
    let (_dir, writer) = setup();
    let reader = writer.reader();
    let all = window(&reader, 1, None, 0, 19);
    let forward = reader
        .read_slice_checked(
            read(1, false),
            SliceRequest {
                navigation: Navigation::After(all.entries[18].key.clone()),
                limit: 5,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&forward), ["20"]);
    let backward = reader
        .read_slice_checked(
            read(1, false),
            SliceRequest {
                navigation: Navigation::Before(all.entries[1].key.clone()),
                limit: 5,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&backward), ["01"]);
}

fn seed(
    anchor: Option<ReadPosition>,
    before: u32,
    after: u32,
    snapshot: Option<SnapshotId>,
) -> SliceRequest {
    SliceRequest {
        navigation: anchor.map(Navigation::Around).unwrap_or(Navigation::Head),
        limit: before.saturating_add(after).saturating_add(1),
        snapshot,
    }
}

#[test]
fn strict_keys_paginate_every_sort_without_repeating_or_following_repriced_orders() {
    let (_directory, mut writer) = setup();
    for sort in 0..=6 {
        let mut request = read(1, false);
        request.sort = sort;
        let reader = writer.reader();
        let mut navigation = Navigation::Head;
        let mut observed = vec![];
        loop {
            let batch = reader
                .read_slice_checked(
                    request.clone(),
                    SliceRequest {
                        navigation,
                        limit: 3,
                        snapshot: None,
                    },
                    |_, _| Ok(()),
                    |_| Ok(true),
                )
                .unwrap();
            observed.extend(ids(&batch));
            let Some(next) = batch.next else {
                break;
            };
            navigation = Navigation::After(next);
        }
        assert_eq!(observed.len(), 20);
        assert_eq!(
            observed
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            20
        );
    }
    let reader = writer.reader();
    let request = {
        let mut r = read(1, false);
        r.sort = 1;
        r
    };
    let head = reader
        .read_slice_checked(
            request.clone(),
            SliceRequest {
                navigation: Navigation::Head,
                limit: 3,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    let key = head.next.unwrap();
    writer
        .commit(
            vec![upsert(entity(E::Order(order("03", "19.5", None))))],
            "reprice".into(),
        )
        .unwrap();
    let after = reader
        .read_slice_checked(
            request.clone(),
            SliceRequest {
                navigation: Navigation::After(key.clone()),
                limit: 3,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&after), ["04", "05", "06"]);
    let before = reader
        .read_slice_checked(
            request,
            SliceRequest {
                navigation: Navigation::Before(key),
                limit: 3,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&before), ["01", "02"]);
}

#[test]
fn bounded_empty_slices_return_a_progress_key_without_claiming_unscanned_coverage() {
    let (_dir, mut writer) = setup();
    writer
        .commit(
            (1..=10_002)
                .map(|i| {
                    upsert(entity(E::Order(order(
                        &format!("{i:05}"),
                        &i.to_string(),
                        None,
                    ))))
                })
                .collect(),
            "sparse".into(),
        )
        .unwrap();
    let reader = writer.reader();
    let mut query = read(1, false);
    query.sort = 1;
    let include =
        |e: &w::Entity| Ok(state::order(e).unwrap().price.parse::<u32>().unwrap() > 10_000);
    let first = reader
        .read_slice_checked(
            query.clone(),
            SliceRequest {
                navigation: Navigation::Head,
                limit: 2,
                snapshot: None,
            },
            |_, _| Ok(()),
            include,
        )
        .unwrap();
    assert!(first.entries.is_empty());
    let page = reader
        .query_range_checked(
            query.clone(),
            RangeRequest {
                options: Default::default(),
                range: KeyRange {
                    start: None,
                    end: None,
                },
                limit: 2,
                snapshot: None,
            },
            false,
            |_, _| Ok(()),
            include,
        )
        .unwrap();
    assert!(page.entries.is_empty());
    let boundary = page
        .range
        .end
        .clone()
        .expect("An empty scan must advance its coverage");
    let after = reader
        .query_range_checked(
            query.clone(),
            RangeRequest {
                options: Default::default(),
                range: KeyRange {
                    start: Some(boundary),
                    end: None,
                },
                limit: 2,
                snapshot: None,
            },
            false,
            |_, _| Ok(()),
            include,
        )
        .unwrap();
    assert_eq!(ids(&after), ["10001", "10002"]);
    assert!(after.range.end.is_none());
    assert!(first.next.is_some());
    assert_eq!(
        first.range.end,
        first.next.as_ref().map(ReadPosition::after)
    );
    let complete = reader
        .read_range_checked(query.clone(), first.range, |_, _| Ok(()), include)
        .unwrap();
    assert!(complete.entries.is_empty());
    let second = reader
        .read_slice_checked(
            query,
            SliceRequest {
                navigation: Navigation::After(first.next.unwrap()),
                limit: 2,
                snapshot: None,
            },
            |_, _| Ok(()),
            include,
        )
        .unwrap();
    assert_eq!(ids(&second), ["10001", "10002"]);
    assert!(second.next.is_none());
}

#[test]
fn serialized_compact_keys_preserve_bidirectional_pages_around_reads_and_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "serialized-keys").unwrap();
    let mut changes = catalog();
    for history in [false, true] {
        for i in 0..18 {
            let mut value = order(
                &format!("{}-{i:02}", if history { "h" } else { "a" }),
                ["1.0000000000000000000000000001", "2", "10.000000001"][i % 3],
                history.then_some(100 + (i / 3) as i64),
            );
            value.observed_at_ms = 100 + (i / 4) as i64;
            if i % 4 != 0 {
                value.attributes.insert(
                    "tg.number".into(),
                    w::AttributePredicate {
                        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                            value: Some(w::attribute_value::Value::Integer((i % 5) as i64)),
                        })),
                    },
                );
            }
            changes.push(upsert(entity(E::Order(value))));
        }
    }
    let mut builder = writer.begin_snapshot().unwrap();
    builder.append(&changes).unwrap();
    writer
        .install_snapshot(builder.finish("serialized").unwrap())
        .unwrap();
    let reader = writer.reader();
    let wire = |key: &ReadPosition| ReadPosition::decode(&key.encode()).unwrap();
    for history in [false, true] {
        for sort in 0..=6 {
            let mut request = read(1, history);
            request.sort = sort;
            let full = reader
                .read_slice_checked(
                    request.clone(),
                    SliceRequest {
                        navigation: Navigation::Head,
                        limit: 200,
                        snapshot: None,
                    },
                    |_, _| Ok(()),
                    |_| Ok(true),
                )
                .unwrap();
            assert_eq!(full.entries.len(), 18);
            let expected = ids(&full);
            for backwards in [false, true] {
                let mut navigation = if backwards {
                    Navigation::Before(wire(&full.entries.last().unwrap().key.after()))
                } else {
                    Navigation::Head
                };
                let mut pages = Vec::new();
                loop {
                    assert!(pages.len() < 20, "navigation must make progress");
                    let page = reader
                        .read_slice_checked(
                            request.clone(),
                            SliceRequest {
                                navigation,
                                limit: 4,
                                snapshot: None,
                            },
                            |_, _| Ok(()),
                            |_| Ok(true),
                        )
                        .unwrap();
                    pages.push(ids(&page));
                    let Some(next) = page.next else {
                        break;
                    };
                    navigation = if backwards {
                        Navigation::Before(wire(&next))
                    } else {
                        Navigation::After(wire(&next))
                    };
                }
                if backwards {
                    pages.reverse();
                }
                assert_eq!(
                    pages.into_iter().flatten().collect::<Vec<_>>(),
                    expected,
                    "sort={sort} history={history} backwards={backwards}"
                );
            }
            let anchor = &full.entries[8].key;
            let around = reader
                .read_slice_checked(
                    request.clone(),
                    SliceRequest {
                        navigation: Navigation::Around(wire(anchor)),
                        limit: 5,
                        snapshot: None,
                    },
                    |_, _| Ok(()),
                    |_| Ok(true),
                )
                .unwrap();
            assert!(around.entries.iter().any(|row| &row.key == anchor));
            let range = reader
                .read_range_checked(
                    request.clone(),
                    KeyRange {
                        start: Some(wire(&full.entries[4].key)),
                        end: Some(wire(&full.entries[10].key)),
                    },
                    |_, _| Ok(()),
                    |_| Ok(true),
                )
                .unwrap();
            assert_eq!(ids(&range), expected[4..10]);
            // Filter changes reuse the same numeric/identity boundary, including
            // when the original row no longer matches the new selection.
            let mut narrower = request.clone();
            let Some(w::selection::Source::Scope(scope)) =
                narrower.selection.as_mut().unwrap().source.as_mut()
            else {
                unreachable!()
            };
            scope.price = Some(w::PriceRange {
                min: Some("2".into()),
                max: None,
            });
            let matching = |row: &&ReadEntry| {
                state::decimal(&state::order(&row.entity).unwrap().price).unwrap()
                    >= Decimal::from(2)
            };
            let resumed = reader
                .read_slice_checked(
                    narrower.clone(),
                    SliceRequest {
                        navigation: Navigation::After(wire(anchor)),
                        limit: 200,
                        snapshot: None,
                    },
                    |_, _| Ok(()),
                    |_| Ok(true),
                )
                .unwrap();
            let expected_after = full
                .entries
                .iter()
                .filter(matching)
                .filter(|row| row.key > *anchor)
                .map(|row| state::order(&row.entity).unwrap().id.clone())
                .collect::<Vec<_>>();
            assert_eq!(
                ids(&resumed),
                expected_after,
                "sort={sort}, history={history}"
            );
            let narrowed_range = reader
                .read_range_checked(
                    narrower,
                    KeyRange {
                        start: Some(wire(&full.entries[4].key)),
                        end: Some(wire(&full.entries[10].key)),
                    },
                    |_, _| Ok(()),
                    |_| Ok(true),
                )
                .unwrap();
            let expected_range = full.entries[4..10]
                .iter()
                .filter(matching)
                .map(|row| state::order(&row.entity).unwrap().id.clone())
                .collect::<Vec<_>>();
            assert_eq!(ids(&narrowed_range), expected_range);
            for row in &narrowed_range.entries {
                assert_eq!(
                    row.key,
                    full.entries
                        .iter()
                        .find(|original| original.key.key == row.key.key)
                        .unwrap()
                        .key
                );
            }
            // Direction, ranking kind and source remain part of key compatibility.
            for wrong_sort in 0..=6 {
                if wrong_sort == sort {
                    continue;
                }
                let mut other = request.clone();
                other.sort = wrong_sort;
                let error = reader
                    .read_slice_checked(
                        other,
                        SliceRequest {
                            navigation: Navigation::After(wire(anchor)),
                            limit: 4,
                            snapshot: None,
                        },
                        |_, _| Ok(()),
                        |_| Ok(true),
                    )
                    .err()
                    .unwrap();
                assert_eq!(error.code(), tonic::Code::InvalidArgument);
            }
            for modes in [vec![2], vec![1, 2], vec![if history { 1 } else { 3 }]] {
                let mut other = request.clone();
                let Some(w::selection::Source::Scope(scope)) =
                    other.selection.as_mut().unwrap().source.as_mut()
                else {
                    unreachable!()
                };
                scope.modes = modes;
                let error = reader
                    .read_slice_checked(
                        other,
                        SliceRequest {
                            navigation: Navigation::After(wire(anchor)),
                            limit: 4,
                            snapshot: None,
                        },
                        |_, _| Ok(()),
                        |_| Ok(true),
                    )
                    .err()
                    .unwrap();
                assert_eq!(error.code(), tonic::Code::InvalidArgument);
            }
        }
    }
}

#[test]
fn range_queries_page_without_duplicates_and_watches_reject_incomplete_coverage() {
    let (_dir, writer) = setup();
    let reader = writer.reader();
    for sort in 0..=6 {
        let mut request = read(1, false);
        request.sort = sort;
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
                        options: Default::default(),
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
            let Some(next) = page.next else {
                break;
            };
            assert_eq!(page.range.end.as_ref(), Some(&next));
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
                    options: Default::default(),
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
        assert!(complete.next.is_none());
        assert_eq!(
            reader
                .query_range_checked(
                    request,
                    RangeRequest {
                        options: Default::default(),
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
            let mut request = read(1, history);
            request.sort = sort;
            let full = reader
                .query_range_checked(
                    request.clone(),
                    RangeRequest {
                        options: Default::default(),
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
                            options: Default::default(),
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
                        options: Default::default(),
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
