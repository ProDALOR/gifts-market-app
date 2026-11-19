use super::*;
use crate::test_support::{catalog, entity, order, read, upsert};
use crate::types::{Key, Sort};
use rust_decimal::Decimal;
use w::entity::Entity as E;
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
    anchor: Option<String>,
    before: u32,
    after: u32,
) -> WindowReadResult {
    let mut request = read(1, false);
    request.sort = sort;
    reader
        .read_window_checked(
            request,
            WindowRequest {
                fill_edges: true,
                anchor,
                before,
                after,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap()
}
fn ids(window: &WindowReadResult) -> Vec<String> {
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
                .all(|pair| pair[0].sort_key < pair[1].sort_key)
        );
        assert!(!first.has_before);
        assert!(first.has_after);
        let middle = window(&reader, sort, Some(first.entries[4].position.clone()), 2, 2);
        assert_eq!(middle.entries[2].position, first.entries[4].position);
        assert!(middle.has_before);
        assert!(middle.has_after);
        let back = window(
            &reader,
            sort,
            Some(middle.entries[0].position.clone()),
            2,
            2,
        );
        assert_eq!(ids(&back), ids(&first));
        let mut last = middle;
        while last.has_after {
            last = window(&reader, sort, Some(last.entries[4].position.clone()), 2, 2);
        }
        assert_eq!(last.entries.len(), 5);
        assert!(last.has_before);
        assert!(!last.has_after);
    }
}
#[test]
fn inserts_removals_and_price_moves_preserve_identity_and_nearest_fallback() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let initial = window(&reader, 1, None, 2, 2);
    let anchor = initial.entries[4].position.clone();
    let old = window(&reader, 1, Some(anchor.clone()), 2, 2);
    assert_eq!(ids(&old), ["03", "04", "05", "06", "07"]);
    writer
        .commit(
            vec![upsert(entity(E::Order(order("new", "4.5", None))))],
            "insert".into(),
        )
        .unwrap();
    let next = window(&reader, 1, Some(anchor.clone()), 2, 2);
    assert_eq!(ids(&next), ["04", "new", "05", "06", "07"]);
    writer
        .commit(
            vec![upsert(entity(E::Order(order("05", "15.5", None))))],
            "move".into(),
        )
        .unwrap();
    let next = window(&reader, 1, Some(anchor), 2, 2);
    assert_eq!(ids(&next), ["14", "15", "05", "16", "17"]);
    let moved = next.entries[2].position.clone();
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
    assert_eq!(ids(&next), ["14", "15", "16", "17", "18"]);
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
        .read_window_checked(
            request,
            WindowRequest {
                fill_edges: true,
                anchor: None,
                before: 2,
                after: 2,
                snapshot: None,
            },
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
fn anchors_are_query_bound_and_limits_and_cancellation_are_enforced() {
    let (_dir, writer) = setup();
    let reader = writer.reader();
    let first = window(&reader, 1, None, 2, 2);
    let mut request = read(1, false);
    request.sort = 2;
    assert!(
        reader
            .read_window_checked(
                request,
                WindowRequest {
                    fill_edges: true,
                    anchor: Some(first.entries[0].position.clone()),
                    before: 2,
                    after: 2,
                    snapshot: None,
                },
                |_, _| Ok(()),
                |_| Ok(true)
            )
            .is_err()
    );
    for (before, after) in [(200, 0), (u32::MAX, 1)] {
        assert!(
            reader
                .read_window_checked(
                    read(1, false),
                    WindowRequest {
                        fill_edges: true,
                        anchor: None,
                        before,
                        after,
                        snapshot: None,
                    },
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
            .read_window_checked(
                read(1, false),
                WindowRequest {
                    fill_edges: true,
                    anchor: None,
                    before: 1,
                    after: 1,
                    snapshot: None,
                },
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
        .read_window_checked(
            request,
            WindowRequest {
                fill_edges: true,
                anchor: None,
                before: 2,
                after: 2,
                snapshot: None,
            },
            |_, _| Ok(()),
            |e| Ok(state::order(e).unwrap().quantity != "0"),
        )
        .unwrap();
    assert_eq!(ids(&result), ["19", "18", "17", "16", "15"]);
    assert!(result.has_after);
}
#[test]
fn empty_history_seeds_and_oversized_reads_do_not_return_partial_results() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    assert!(
        reader
            .read_window_checked(
                read(1, true),
                WindowRequest {
                    fill_edges: true,
                    anchor: None,
                    before: 1,
                    after: 1,
                    snapshot: None,
                },
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
        .read_window_checked(
            read(1, false),
            WindowRequest {
                fill_edges: true,
                anchor: None,
                before: 1,
                after: 1,
                snapshot: None,
            },
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
    let anchor = live.entries[15].position.clone();
    let frozen = reader
        .read_window_checked(
            read(0, false),
            WindowRequest {
                fill_edges: true,
                anchor: Some(anchor.clone()),
                before: 2,
                after: 2,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&frozen), ["14", "15", "16", "17", "18"]);
    let lease = frozen.snapshot.clone().unwrap();
    writer
        .apply(w::Batch {
            cursor: "moved".into(),
            changes: vec![upsert(entity(E::Order(order("16", "0.1", None))))],
        })
        .unwrap();
    let same = reader
        .read_window_checked(
            read(0, false),
            WindowRequest {
                fill_edges: true,
                anchor: Some(anchor.clone()),
                before: 2,
                after: 2,
                snapshot: Some(lease.clone()),
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&same), ids(&frozen));
    assert_eq!(same.revision, frozen.revision);
    assert_ne!(ids(&window(&reader, 1, Some(anchor), 2, 2)), ids(&frozen));
    reader.release_snapshot(&lease).unwrap();
    reader.release_snapshot(&lease).unwrap();
    assert!(!reader.0.views.lock().unwrap().contains_key(&lease));
    let error = reader
        .read_window_checked(
            read(0, false),
            WindowRequest {
                fill_edges: true,
                anchor: Some(same.entries[2].position.clone()),
                before: 2,
                after: 2,
                snapshot: Some(lease),
            },
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
        .read_window_checked(
            read(0, false),
            WindowRequest {
                fill_edges: true,
                anchor: None,
                before: 2,
                after: 2,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    let lease = first.snapshot.unwrap();
    let result = reader.read_window_checked(
        read(0, false),
        WindowRequest {
            fill_edges: true,
            anchor: Some(first.entries[0].position.clone()),
            before: 2,
            after: 2,
            snapshot: Some(lease.clone()),
        },
        |_, _| Ok(()),
        |_| {
            reader.release_snapshot(&lease)?;
            Ok(true)
        },
    );
    assert!(result.is_err());
    assert!(!reader.0.views.lock().unwrap().contains_key(&lease));
}

#[test]
fn public_ordering_keys_preserve_exact_rank_identity_prefixes_and_direction() {
    let key = Key::order("v".into(), "a".into());
    assert_eq!(
        super::window::ordering_key(Decimal::ZERO, &key, Sort::PriceAsc),
        super::window::ordering_key(Decimal::from_str_exact("-0").unwrap(), &key, Sort::PriceAsc)
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
            .map(|(rank, key)| super::window::ordering_key(rank, &key, sort))
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
        .read_window_checked(
            read(1, false),
            WindowRequest {
                fill_edges: true,
                anchor: None,
                before: 0,
                after: 0,
                snapshot: Some("lease".into()),
            },
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
    let anchor = all.entries.last().unwrap().position.clone();
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
    assert!(!empty.has_before && !empty.has_after);
}

#[test]
fn fixed_ranges_use_original_ordering_bounds_after_anchor_repricing() {
    let (_dir, mut writer) = setup();
    let reader = writer.reader();
    let all = window(&reader, 1, None, 0, 19);
    let coverage = WindowRange {
        start: Some(all.entries[9].position.clone()),
        end: Some(all.entries[14].position.clone()),
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
    assert_eq!(ids(&current), ["new", "11", "12", "13", "14", "15"]);
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
            .read_window_checked(
                query.clone(),
                WindowRequest {
                    fill_edges: true,
                    anchor: None,
                    before: 1,
                    after: 1,
                    snapshot: None,
                },
                |_, _| Ok(()),
                |_| Ok(true),
            )
            .unwrap();
        assert_eq!(slice.entries.len(), 3);
        assert!(
            slice
                .entries
                .windows(2)
                .all(|pair| pair[0].sort_key < pair[1].sort_key)
        );
        let singleton = reader
            .read_range_checked(
                query.clone(),
                WindowRange {
                    start: Some(slice.entries[1].position.clone()),
                    end: Some(slice.entries[1].position.clone()),
                },
                |_, _| Ok(()),
                |_| Ok(true),
            )
            .unwrap();
        assert_eq!(ids(&singleton), vec![ids(&slice)[1].clone()]);
        let middle = reader
            .read_window_checked(
                query,
                WindowRequest {
                    fill_edges: true,
                    anchor: Some(slice.entries[1].position.clone()),
                    before: 1,
                    after: 1,
                    snapshot: None,
                },
                |_, _| Ok(()),
                |_| Ok(true),
            )
            .unwrap();
        assert_eq!(ids(&middle), ids(&slice));
    }
    let mut query = read(1, true);
    query.sort = w::ReadSort::TimeDesc as i32;
    let slice = reader
        .read_window_checked(
            query.clone(),
            WindowRequest {
                fill_edges: true,
                anchor: None,
                before: 0,
                after: 1,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&slice), ["h2", "h4"]);
    let coverage = WindowRange {
        start: None,
        end: Some(slice.entries[1].position.clone()),
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
            .all(|pair| pair[0].sort_key < pair[1].sort_key)
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
        .read_window_checked(
            read(1, false),
            WindowRequest {
                fill_edges: false,
                anchor: Some(all.entries[18].position.clone()),
                before: 0,
                after: 4,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&forward), ["19", "20"]);
    let backward = reader
        .read_window_checked(
            read(1, false),
            WindowRequest {
                fill_edges: false,
                anchor: Some(all.entries[1].position.clone()),
                before: 4,
                after: 0,
                snapshot: None,
            },
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap();
    assert_eq!(ids(&backward), ["01", "02"]);
}
