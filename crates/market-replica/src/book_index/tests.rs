use super::*;
use crate::{
    state::{State, entity_key},
    test_support::{entity, order},
};
use w::entity::Entity as E;

fn scope(book: &w::BookKey) -> Scope {
    w::Scope {
        item_type_id: Some(book.item_type_id.clone()),
        currency_id: Some(book.currency_id.clone()),
        ..Default::default()
    }
    .try_into()
    .unwrap()
}

#[test]
fn enum_candidates_preserve_rank_ties_venue_isolation_shared_records_and_updates() {
    let mut state = State::default();
    for (id, venue, kind, price) in [
        ("a", "one", "rare", "1"),
        ("b", "two", "rare", "1"),
        ("c", "one", "common", "1"),
        ("d", "two", "rare", "2"),
    ] {
        let mut row = order(id, price, None);
        row.book.as_mut().unwrap().venue_id = venue.into();
        row.attributes.insert(
            "model".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::OptionId(kind.into())),
                })),
            },
        );
        state.upsert(entity(E::Order(row))).unwrap();
    }
    let mut selected = scope(&order("", "1", None).book.unwrap());
    selected.attributes.insert(
        "model".into(),
        w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                value: Some(w::attribute_value::Value::OptionId("rare".into())),
            })),
        },
    );
    for venue in [None, Some("one"), Some("two")] {
        selected.venue_id = venue.map(str::to_owned);
        for sort in [
            Sort::PriceAsc,
            Sort::PriceDesc,
            Sort::TimeAsc,
            Sort::TimeDesc,
            Sort::NumberAsc,
            Sort::NumberDesc,
        ] {
            let mut full = selected.clone();
            full.attributes.clear();
            let expected: Vec<_> = state
                .books()
                .ranked(&full, None, sort)
                .filter(|(_, entity)| {
                    crate::state::order(entity).unwrap().attributes["model"]
                        == selected.attributes["model"]
                })
                .collect();
            let actual: Vec<_> = state.books().ranked(&selected, None, sort).collect();
            assert_eq!(actual, expected);
            for (i, (rank, record)) in actual.iter().enumerate() {
                assert!(Arc::ptr_eq(
                    record,
                    state.entity(&entity_key(record).unwrap()).unwrap()
                ));
                let after: Vec<_> = state
                    .books()
                    .ranked(&selected, Some((*rank).clone()), sort)
                    .collect();
                assert_eq!(after, actual[i + 1..]);
            }
        }
    }
    selected.venue_id = None;
    let pinned = state.clone();
    let mut changed = order("a", "1", None);
    changed.book.as_mut().unwrap().venue_id = "one".into();
    changed.attributes.insert(
        "model".into(),
        w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                value: Some(w::attribute_value::Value::OptionId("common".into())),
            })),
        },
    );
    state.upsert(entity(E::Order(changed))).unwrap();
    assert_eq!(
        state
            .books()
            .ranked(&selected, None, Sort::PriceAsc)
            .count(),
        2
    );
    assert_eq!(
        pinned
            .books()
            .ranked(&selected, None, Sort::PriceAsc)
            .count(),
        3
    );
    assert_shared_records(&state, 4);
}

fn assert_shared_records(state: &State, expected: usize) {
    assert_eq!(state.books().locations.len(), expected);
    let mut stored = 0;
    for (location, partition) in &state.books().partitions {
        stored += partition.orders.len();
        for (id, record) in &partition.orders {
            let key = entity_key(record).unwrap();
            assert_eq!(id, &key.id);
            assert!(Arc::ptr_eq(
                state.books().locations.get(&key).unwrap(),
                location
            ));
            assert!(Arc::ptr_eq(state.entity(&key).unwrap(), record));
        }
        for sort in [Sort::PriceAsc, Sort::TimeAsc, Sort::NumberAsc] {
            assert_eq!(partition.index(sort).len(), partition.orders.len());
        }
    }
    assert_eq!(stored, expected);
    for sort in [Sort::PriceAsc, Sort::TimeAsc, Sort::NumberAsc] {
        let mut count = 0;
        for (partition, indexes) in &state.books().partitions {
            assert!(!indexes.is_empty(), "empty partitions must be removed");
            for (rank, indexed) in indexes.index(sort) {
                let (value, key) = rank.entity();
                let authoritative = state.entity(key).unwrap();
                assert!(Arc::ptr_eq(indexed, authoritative));
                assert_eq!(entity_key(indexed).unwrap(), *key);
                let order = crate::state::order(indexed).unwrap();
                assert_eq!(value, sort.rank(order, false).unwrap());
                assert!(partition.book == Book::from(order.book.as_ref().unwrap()));
                assert!(partition.mode == ActiveMode::try_from(order.mode).unwrap());
                count += 1;
            }
        }
        assert_eq!(count, expected);
    }
}

#[test]
fn ranked_indexes_share_one_payload_and_refresh_same_rank_updates() {
    let mut state = State::default();
    let original = entity(E::Order(order("same", "1", None)));
    state.upsert(original.clone()).unwrap();
    assert_shared_records(&state, 1);
    let key = entity_key(&original).unwrap();
    // The authoritative map plus three covering indexes own the same allocation.
    assert_eq!(Arc::strong_count(state.entity(&key).unwrap()), 4);
    let pinned = state.clone();
    let mut updated = order("same", "1.0", None);
    updated.meta.insert("label".into(), "new payload".into());
    state.upsert(entity(E::Order(updated.clone()))).unwrap();
    assert_shared_records(&state, 1);
    assert_shared_records(&pinned, 1);
    assert!(state.books().references.ptr_eq(&pinned.books().references));
    assert!(state.books().groups.ptr_eq(&pinned.books().groups));
    assert!(state.books().declared.ptr_eq(&pinned.books().declared));
    assert_eq!(state.books().partitions.len(), 1);
    assert!(!Arc::ptr_eq(
        state.entity(&key).unwrap(),
        pinned.entity(&key).unwrap()
    ));
    for sort in [Sort::PriceAsc, Sort::TimeAsc, Sort::NumberAsc] {
        let selection = scope(updated.book.as_ref().unwrap());
        let (_, current) = state.books().ranked(&selection, None, sort).next().unwrap();
        assert_eq!(crate::state::order(current).unwrap(), &updated);
        let (_, previous) = pinned
            .books()
            .ranked(&selection, None, sort)
            .next()
            .unwrap();
        assert_eq!(previous.as_ref(), &original);
    }
}

#[test]
fn ranked_records_follow_partition_moves_removals_and_terminal_transitions() {
    let mut state = State::default();
    let mut updated = order("moving", "1", None);
    let original_book = updated.book.clone().unwrap();
    state.upsert(entity(E::Order(updated.clone()))).unwrap();
    let pinned = state.clone();
    updated.price = "123.000000001".into();
    updated.observed_at_ms = 9999;
    updated.mode = 2;
    updated.book.as_mut().unwrap().item_type_id = "another".into();
    updated.book.as_mut().unwrap().currency_id = "XTR".into();
    updated.attributes.insert(
        "tg.number".into(),
        w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                value: Some(w::attribute_value::Value::Integer(42)),
            })),
        },
    );
    let moved = entity(E::Order(updated.clone()));
    let key = entity_key(&moved).unwrap();
    state.upsert(moved.clone()).unwrap();
    assert_shared_records(&state, 1);
    assert_shared_records(&pinned, 1);
    assert!(!state.books().contains(&original_book));
    for sort in [Sort::PriceAsc, Sort::TimeAsc, Sort::NumberAsc] {
        assert!(
            state
                .books()
                .ranked(&scope(&original_book), None, sort)
                .next()
                .is_none()
        );
    }
    // A terminal update removes the current record from every covering index.
    updated.mode = 3;
    updated.executed_at_ms = Some(10000);
    state.upsert(entity(E::Order(updated))).unwrap();
    assert_shared_records(&state, 0);
    assert!(state.entity(&key).is_none());
    state.upsert(moved).unwrap();
    state.remove(&key).unwrap();
    state.remove(&key).unwrap();
    assert_shared_records(&state, 0);
    assert_shared_records(&pinned, 1);
    assert!(state.books().references.is_empty());
    assert!(state.books().groups.is_empty());
}

#[test]
fn borrowed_ranked_merge_keeps_cross_venue_ties_ranges_and_boundaries() {
    let mut state = State::default();
    for venue in ["a", "b", "c"] {
        for (id, price, mode) in [("same", "2", 1), ("other", "2.00", 2), ("outside", "3", 1)] {
            let mut value = order(id, price, None);
            value.book.as_mut().unwrap().venue_id = venue.into();
            value.mode = mode;
            state.upsert(entity(E::Order(value))).unwrap();
        }
    }
    assert_shared_records(&state, 9);
    let mut selection = scope(order("", "", None).book.as_ref().unwrap());
    selection.price = Some(crate::query::ExactPriceRange {
        min: Some(2.into()),
        max: Some(2.into()),
    });
    for sort in [
        Sort::PriceAsc,
        Sort::PriceDesc,
        Sort::TimeAsc,
        Sort::TimeDesc,
        Sort::NumberAsc,
        Sort::NumberDesc,
    ] {
        let expected: Vec<_> = state
            .books()
            .ranked(&selection, None, sort)
            .map(|(rank, record)| {
                assert!(Arc::ptr_eq(record, state.entity(rank.entity().1).unwrap()));
                rank.clone()
            })
            .collect();
        assert_eq!(expected.len(), if sort.is_price() { 6 } else { 9 });
        assert!(expected.windows(2).all(|w| if sort.descending() {
            w[0] > w[1]
        } else {
            w[0] < w[1]
        }));
        let mut actual = Vec::new();
        let mut after = None;
        loop {
            let page: Vec<_> = state
                .books()
                .ranked(&selection, after, sort)
                .take(2)
                .map(|(rank, _)| rank.clone())
                .collect();
            if page.is_empty() {
                break;
            }
            after = page.last().cloned();
            actual.extend(page);
        }
        assert_eq!(actual, expected);
    }
    selection.venue_id = Some("b".into());
    selection.modes.insert(crate::types::OrderMode::Ask);
    let rows: Vec<_> = state
        .books()
        .ranked(&selection, None, Sort::PriceAsc)
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0.entity().1.venue_id, "b");
    assert_eq!(rows[0].0.entity().1.id, "same");
}

fn set_number(order: &mut w::Order, number: Option<i64>) {
    order.attributes.remove("tg.number");
    if let Some(number) = number {
        order.attributes.insert(
            "tg.number".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::Integer(number)),
                })),
            },
        );
    }
}

#[test]
fn direct_replacement_updates_each_rank_without_changing_directory_or_other_partitions() {
    let mut state = State::default();
    let mut a = order("a", "1", None);
    a.observed_at_ms = 10;
    set_number(&mut a, Some(10));
    let mut b = order("b", "2", None);
    b.observed_at_ms = 20;
    set_number(&mut b, Some(20));
    let mut unrelated = order("unrelated", "3", None);
    unrelated.book.as_mut().unwrap().currency_id = "XTR".into();
    for value in [&a, &b, &unrelated] {
        state.upsert(entity(E::Order(value.clone()))).unwrap();
    }
    let other = OrderPlacement::new(&unrelated).unwrap().partition;
    let selection = scope(a.book.as_ref().unwrap());
    for (price, time, number, expected) in [
        ("3", 10, Some(10), [["b", "a"], ["a", "b"], ["a", "b"]]),
        ("3", 30, Some(10), [["b", "a"], ["b", "a"], ["a", "b"]]),
        ("3", 30, Some(30), [["b", "a"], ["b", "a"], ["b", "a"]]),
        ("0.5", 5, None, [["a", "b"], ["a", "b"], ["b", "a"]]),
    ] {
        let pinned = state.clone();
        a.price = price.into();
        a.observed_at_ms = time;
        set_number(&mut a, number);
        state.upsert(entity(E::Order(a.clone()))).unwrap();
        assert_shared_records(&state, 3);
        assert_shared_records(&pinned, 3);
        assert!(state.books().groups.ptr_eq(&pinned.books().groups));
        assert!(state.books().references.ptr_eq(&pinned.books().references));
        assert!(state.books().declared.ptr_eq(&pinned.books().declared));
        assert_eq!(state.books().partitions.len(), 2);
        assert!(state.books().locations.ptr_eq(&pinned.books().locations));
        assert!(Arc::ptr_eq(
            state.books().partitions.get(&other).unwrap(),
            pinned.books().partitions.get(&other).unwrap(),
        ));
        for (sort, expected) in [Sort::PriceAsc, Sort::TimeAsc, Sort::NumberAsc]
            .into_iter()
            .zip(expected)
        {
            let records: Vec<_> = state
                .books()
                .ranked(&selection, None, sort)
                .map(|(_, record)| crate::state::order(record).unwrap().id.as_str())
                .collect();
            assert_eq!(records, expected);
            assert!(
                state
                    .books()
                    .partitions
                    .get(&other)
                    .unwrap()
                    .index(sort)
                    .ptr_eq(pinned.books().partitions.get(&other).unwrap().index(sort))
            );
        }
    }
}

#[test]
fn invalid_replacements_leave_entities_indexes_and_directory_unchanged() {
    let mut state = State::default();
    let original = order("a", "1", None);
    state.upsert(entity(E::Order(original.clone()))).unwrap();
    let pinned = state.clone();
    let mut bad_price = original.clone();
    bad_price.price = "invalid".into();
    let mut bad_mode = original.clone();
    bad_mode.mode = 999;
    let mut missing_book = original.clone();
    missing_book.book = None;
    let mut bad_move = bad_price.clone();
    bad_move.book.as_mut().unwrap().currency_id = "XTR".into();
    for invalid in [bad_price, bad_mode, missing_book, bad_move] {
        assert!(state.upsert(entity(E::Order(invalid))).is_err());
        let before: Vec<_> = pinned.range(None, None).collect();
        let after: Vec<_> = state.range(None, None).collect();
        assert_eq!(before.len(), after.len());
        assert!(
            before
                .iter()
                .zip(after)
                .all(|((key, entity), (other_key, other))| key == &other_key
                    && Arc::ptr_eq(entity, other))
        );
        assert!(state.books().partitions.ptr_eq(&pinned.books().partitions));
        assert!(state.books().locations.ptr_eq(&pinned.books().locations));
        assert!(state.books().references.ptr_eq(&pinned.books().references));
        assert!(state.books().groups.ptr_eq(&pinned.books().groups));
        assert_shared_records(&state, 1);
    }
}

#[test]
fn partition_replacement_keeps_feed_references_and_removes_empty_partitions() {
    let mut state = State::default();
    let mut value = order("moving", "1", None);
    let primary = value.book.clone().unwrap();
    let mut second_book = primary.clone();
    second_book.currency_id = "XTR".into();
    state
        .upsert(entity(E::Feed(w::Feed {
            id: "feed".into(),
            books: vec![primary.clone(), second_book.clone()],
            ..Default::default()
        })))
        .unwrap();
    state.upsert(entity(E::Order(value.clone()))).unwrap();
    let active_book = Book::from(&primary);
    let other_book = Book::from(&second_book);
    assert_eq!(state.books().references.get(&active_book), Some(&2));
    assert_eq!(state.books().references.get(&other_book), Some(&1));
    let pinned = state.clone();
    // Moving the last ask to bids preserves exactly one active-book reference.
    value.mode = 2;
    state.upsert(entity(E::Order(value.clone()))).unwrap();
    assert_eq!(state.books().partitions.len(), 1);
    assert_eq!(state.books().references.get(&active_book), Some(&2));
    assert_eq!(state.books().references.get(&other_book), Some(&1));
    // Moving it to another book releases only the old active reference.
    value.book = Some(second_book);
    state.upsert(entity(E::Order(value.clone()))).unwrap();
    assert_eq!(state.books().references.get(&active_book), Some(&1));
    assert_eq!(state.books().references.get(&other_book), Some(&2));
    assert_shared_records(&state, 1);
    assert_shared_records(&pinned, 1);
    value.mode = 3;
    value.executed_at_ms = Some(2000);
    state.upsert(entity(E::Order(value))).unwrap();
    assert!(state.books().partitions.is_empty());
    assert!(state.books().groups.is_empty());
    assert_eq!(state.books().references.get(&active_book), Some(&1));
    assert_eq!(state.books().references.get(&other_book), Some(&1));
    let feed = crate::types::Key {
        kind: crate::types::EntityKind::Feed,
        venue_id: String::new(),
        id: "feed".into(),
    };
    state.remove(&feed).unwrap();
    assert!(state.books().references.is_empty());
}

#[test]
fn locations_share_partition_keys_and_follow_moves_without_retaining_removed_orders() {
    let mut state = State::default();
    let a = entity(E::Order(order("a", "1", None)));
    let b = entity(E::Order(order("b", "2", None)));
    let a_key = entity_key(&a).unwrap();
    let b_key = entity_key(&b).unwrap();
    state.upsert(a).unwrap();
    state.upsert(b).unwrap();
    assert!(Arc::ptr_eq(
        state.books().locations.get(&a_key).unwrap(),
        state.books().locations.get(&b_key).unwrap()
    ));
    let pinned = state.clone();
    let mut updated = order("a", "3", None);
    updated.mode = 2;
    updated.book.as_mut().unwrap().currency_id = "XTR".into();
    state.upsert(entity(E::Order(updated.clone()))).unwrap();
    assert_shared_records(&state, 2);
    assert_shared_records(&pinned, 2);
    assert!(!Arc::ptr_eq(
        state.books().locations.get(&a_key).unwrap(),
        state.books().locations.get(&b_key).unwrap()
    ));
    assert_eq!(
        crate::state::order(state.entity(&a_key).unwrap()).unwrap(),
        &updated
    );
    assert_eq!(
        crate::state::order(pinned.entity(&a_key).unwrap())
            .unwrap()
            .price,
        "1"
    );
    state.remove(&a_key).unwrap();
    state.remove(&a_key).unwrap();
    assert!(state.entity(&a_key).is_none());
    assert!(state.books().locations.get(&a_key).is_none());
    assert_shared_records(&state, 1);
    assert!(pinned.entity(&a_key).is_some());
}

#[test]
fn number_predicates_seek_rank_bounds_in_both_directions_without_losing_ties_or_cursors() {
    let mut state = State::default();
    for (id, number) in [
        ("before", 1),
        ("a", 100_000),
        ("b", 100_000),
        ("after", 200_000),
    ] {
        let mut value = order(id, "1", None);
        set_number(&mut value, Some(number));
        state.upsert(entity(E::Order(value))).unwrap();
    }
    let mut selection = scope(order("", "", None).book.as_ref().unwrap());
    selection.attributes.insert(
        "tg.number".into(),
        w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                value: Some(w::attribute_value::Value::Integer(100_000)),
            })),
        },
    );
    for (sort, expected) in [
        (Sort::NumberAsc, ["a", "b"]),
        (Sort::NumberDesc, ["b", "a"]),
    ] {
        let rows: Vec<_> = state.books().ranked(&selection, None, sort).collect();
        assert_eq!(
            rows.iter()
                .map(|(key, _)| key.entity().1.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        let first = rows[0].0.clone();
        let remainder: Vec<_> = state
            .books()
            .ranked(&selection, Some(first), sort)
            .collect();
        assert_eq!(remainder.len(), 1);
        assert_eq!(remainder[0].0.entity().1.id, expected[1]);
    }
    selection.attributes.insert(
        "tg.number".into(),
        w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::Range(
                w::AttributeRange {
                    min: Some(w::AttributeValue {
                        value: Some(w::attribute_value::Value::Integer(100_000)),
                    }),
                    max: Some(w::AttributeValue {
                        value: Some(w::attribute_value::Value::Integer(200_000)),
                    }),
                },
            )),
        },
    );
    assert_eq!(
        state
            .books()
            .ranked(&selection, None, Sort::NumberAsc)
            .count(),
        3
    );
}

#[test]
fn scoped_order_snapshot_shares_only_selected_partitions_and_survives_replacement() {
    let mut state = State::default();
    for (id, collection, currency, mode, venue) in [
        ("ask", "gift", "GRAM", 1, "venue"),
        ("second", "gift", "GRAM", 1, "second"),
        ("bid", "gift", "GRAM", 2, "venue"),
        ("other-collection", "other", "GRAM", 1, "venue"),
        ("other-currency", "gift", "XTR", 1, "venue"),
    ] {
        let mut row = order(id, "10", None);
        row.mode = mode;
        row.book = Some(w::BookKey {
            venue_id: venue.into(),
            item_type_id: collection.into(),
            currency_id: currency.into(),
        });
        state.upsert(entity(E::Order(row))).unwrap();
    }
    let mut selected = scope(&order("", "1", None).book.unwrap());
    selected.modes.insert(crate::types::OrderMode::Ask);
    let snapshot = OrderBookSnapshot::new(state.books(), selected);
    assert_eq!(snapshot.index.partitions.len(), 2);
    assert!(
        snapshot.index.locations.is_empty(),
        "The scoped view does not retain global identity indexes"
    );
    let rows: Vec<_> = snapshot.candidates(Default::default()).cloned().collect();
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert!(Arc::ptr_eq(
            row,
            state.entity(&entity_key(row).unwrap()).unwrap()
        ));
    }
    let unrelated = Arc::downgrade(
        state
            .entity(&Key::order("venue".into(), "other-collection".into()))
            .unwrap(),
    );
    state
        .upsert(entity(E::Order(order("ask", "1", None))))
        .unwrap();
    assert!(
        snapshot
            .candidates(Default::default())
            .all(|row| crate::state::order(row).unwrap().price == "10")
    );
    assert_eq!(
        crate::state::order(
            state
                .entity(&Key::order("venue".into(), "ask".into()))
                .unwrap()
        )
        .unwrap()
        .price,
        "1"
    );
    drop(state);
    assert!(
        unrelated.upgrade().is_none(),
        "Unrelated orders are released even while the scoped view remains alive"
    );
    assert_eq!(snapshot.candidates(Default::default()).count(), 2);
}
