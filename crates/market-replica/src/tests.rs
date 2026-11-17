use super::*;
use w::entity::Entity as E;
pub(super) fn entity(value: E) -> w::Entity {
    w::Entity {
        entity: Some(value),
    }
}
pub(super) fn upsert(value: w::Entity) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Upsert(value)),
    }
}
pub(super) fn catalog() -> Vec<w::Change> {
    vec![
        upsert(entity(E::Venue(w::Venue {
            id: "venue".into(),
            name: "Venue".into(),
        }))),
        upsert(entity(E::Currency(w::Currency {
            id: "GRAM".into(),
            name: "Gram".into(),
            decimals: 9,
        }))),
        upsert(entity(E::Currency(w::Currency {
            id: "XTR".into(),
            name: "Stars".into(),
            decimals: 9,
        }))),
        upsert(entity(E::ItemType(w::ItemType {
            id: "gift".into(),
            name: "Gift".into(),
            quantity_step: "1".into(),
            attributes: [("stickers".into(), w::AttributeRule::default())].into(),
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "stickers".into(),
            name: "Stickers".into(),
            value_type: 7,
            element: Some("sticker".into()),
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "sticker".into(),
            name: "Sticker".into(),
            value_type: 8,
            fields: [
                ("name".into(), w::AttributeRule::default()),
                ("wear".into(), w::AttributeRule::default()),
            ]
            .into(),
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "name".into(),
            name: "Name".into(),
            value_type: 5,
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "wear".into(),
            name: "Wear".into(),
            value_type: 3,
            ..Default::default()
        }))),
    ]
}
pub(super) fn order(id: &str, price: &str, time: Option<i64>) -> w::Order {
    w::Order {
        id: id.into(),
        book: Some(w::BookKey {
            venue_id: "venue".into(),
            item_type_id: "gift".into(),
            currency_id: "GRAM".into(),
        }),
        mode: if time.is_some() { 3 } else { 1 },
        price: price.into(),
        quantity: "1".into(),
        observed_at_ms: 1000,
        executed_at_ms: time,
        ..Default::default()
    }
}
pub(super) fn read(mode: i32, history: bool) -> w::ReadRequest {
    w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(w::Scope {
                item_type_id: Some("gift".into()),
                currency_id: Some("GRAM".into()),
                modes: vec![if history { 3 } else { 1 }],
                ..Default::default()
            })),
            ..Default::default()
        }),
        mode,
        sort: if history { 0 } else { 1 },
        limit: 1,
        ..Default::default()
    }
}
// Pure state/storage tests use a local writer. Production writes are only exposed
// through the actor's private replication capability (covered in actor tests).
struct Fixture {
    reader: ReplicaReader,
    writer: std::sync::Mutex<ReplicaWriter>,
}
impl Fixture {
    fn open(directory: &std::path::Path, source: &str) -> Result<Self, Status> {
        let writer = ReplicaWriter::open(directory, source)?;
        Ok(Self {
            reader: writer.reader.clone(),
            writer: std::sync::Mutex::new(writer),
        })
    }
    fn apply(&self, batch: w::Batch) -> Result<(), Status> {
        self.writer.lock().unwrap().apply(batch)
    }
    fn install(&self, store: Store) -> Result<(), Status> {
        self.writer.lock().unwrap().install(store)
    }
}
impl std::ops::Deref for Fixture {
    type Target = ReplicaReader;
    fn deref(&self) -> &ReplicaReader {
        &self.reader
    }
}
fn setup() -> (tempfile::TempDir, Fixture) {
    let dir = tempfile::tempdir().unwrap();
    let replica = Fixture::open(dir.path(), "test-source").unwrap();
    let store = replica.writer.lock().unwrap().store.clone();
    store.apply(&catalog(), Some("c0")).unwrap();
    replica.install(store).unwrap();
    (dir, replica)
}
fn apply(fixture: &Fixture, changes: Vec<w::Change>, cursor: &str) {
    fixture
        .apply(w::Batch {
            changes,
            cursor: cursor.into(),
        })
        .unwrap();
}
fn ids(page: &w::Page) -> Vec<String> {
    page.entities
        .iter()
        .map(|e| state::order(e).unwrap().id.clone())
        .collect()
}
#[test]
fn local_price_pages_are_exact_currency_isolated_and_share_live_snapshot_boundaries() {
    let (_dir, replica) = setup();
    let mut stars = order("stars", "0.000001", None);
    stars.book.as_mut().unwrap().currency_id = "XTR".into();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order(
                "z",
                "10000000000000000.000000001",
                None,
            )))),
            upsert(entity(E::Order(order(
                "a",
                "10000000000000000.000000002",
                None,
            )))),
            upsert(entity(E::Order(stars))),
        ],
        "c1",
    );
    let first = replica.read(read(1, false)).unwrap();
    assert_eq!(ids(&first.page), ["z"]);
    let mut query = read(0, false);
    query.page_token = first.page.next_page_token;
    let second = replica.read(query).unwrap();
    assert_eq!(ids(&second.page), ["a"]);
    let mut descending = read(1, false);
    descending.sort = 2;
    assert_eq!(ids(&replica.read(descending).unwrap().page), ["a"]);
    let mut query = read(1, false);
    query.limit = 10;
    let scope = match query.selection.as_mut().unwrap().source.as_mut().unwrap() {
        w::selection::Source::Scope(s) => s,
        _ => unreachable!(),
    };
    scope.price = Some(w::PriceRange {
        min: Some("10000000000000000.000000002".into()),
        max: None,
    });
    assert_eq!(ids(&replica.read(query).unwrap().page), ["a"]);
    let first = replica.read(read(0, false)).unwrap();
    let mut changed = read(1, false);
    changed.sort = 2;
    changed.page_token = first.page.next_page_token;
    assert_eq!(
        replica.read(changed).err().unwrap().code(),
        tonic::Code::InvalidArgument
    );
}
#[test]
fn terminal_transitions_late_backfills_and_snapshots_remain_consistent() {
    let (dir, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("b", "2", Some(20))))),
            upsert(entity(E::Order(order("c", "3", Some(30))))),
        ],
        "c1",
    );
    let initial = replica.read(read(0, true)).unwrap();
    assert_eq!(ids(&initial.page), ["b"]);
    apply(
        &replica,
        vec![upsert(entity(E::Order(order("a", "1", Some(10)))))],
        "c2",
    );
    assert!(
        replica
            .read(read(1, false))
            .unwrap()
            .page
            .entities
            .is_empty()
    );
    let mut continuation = read(0, true);
    continuation.page_token = initial.page.next_page_token;
    assert_eq!(ids(&replica.read(continuation).unwrap().page), ["c"]);
    let mut history = read(0, true);
    history.limit = 20;
    assert_eq!(
        ids(&replica.read(history.clone()).unwrap().page),
        ["a", "b", "c"]
    );
    let period = history.selection.as_mut().unwrap();
    period.executed_at = Some(w::Period {
        from_ms: Some(10),
        until_ms: Some(20),
    });
    assert_eq!(ids(&replica.read(history).unwrap().page), ["a"]);
    // An execution retry advances the durable cursor without adding another payload.
    let mut retry = order("a", "1", Some(10));
    retry.observed_at_ms = 2000;
    apply(&replica, vec![upsert(entity(E::Order(retry)))], "c3");
    drop(replica);
    let reopened = Fixture::open(dir.path(), "test-source").unwrap();
    assert!(reopened.status().ready);
    assert_eq!(reopened.status().state, SyncState::Stale);
    let mut history = read(0, true);
    history.limit = 20;
    let result = reopened.read(history).unwrap();
    assert_eq!(ids(&result.page), ["a", "b", "c"]);
    assert_eq!(result.page.cursor, "c3");
}
#[test]
fn rejected_batches_persist_neither_current_changes_nor_checkpoint() {
    let (dir, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("done", "2", Some(10))))),
        ],
        "good",
    );
    let error = replica
        .apply(w::Batch {
            changes: vec![
                upsert(entity(E::Order(order("a", "9", None)))),
                upsert(entity(E::Order(order("done", "99", Some(10))))),
            ],
            cursor: "bad".into(),
        })
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::DataLoss);
    assert_eq!(
        state::order(&replica.read(read(1, false)).unwrap().page.entities[0])
            .unwrap()
            .price,
        "1"
    );
    drop(replica);
    let restored = Fixture::open(dir.path(), "test-source").unwrap();
    assert_eq!(restored.view().unwrap().cursor, "good");
    assert_eq!(
        state::order(&restored.read(read(1, false)).unwrap().page.entities[0])
            .unwrap()
            .price,
        "1"
    );
}
fn text(value: &str) -> w::AttributeValue {
    w::AttributeValue {
        value: Some(w::attribute_value::Value::Text(value.into())),
    }
}
fn sticker(name: &str, wear: &str) -> w::AttributeValue {
    w::AttributeValue {
        value: Some(w::attribute_value::Value::Object(w::AttributeObject {
            fields: [
                ("name".into(), text(name)),
                (
                    "wear".into(),
                    w::AttributeValue {
                        value: Some(w::attribute_value::Value::Decimal(wear.into())),
                    },
                ),
            ]
            .into(),
        })),
    }
}
fn eq(value: w::AttributeValue) -> w::AttributePredicate {
    w::AttributePredicate {
        predicate: Some(w::attribute_predicate::Predicate::Eq(value)),
    }
}
fn contains(name: &str, wear: &str, min: u32) -> w::AttributePredicate {
    w::AttributePredicate {
        predicate: Some(w::attribute_predicate::Predicate::Contains(Box::new(
            w::ContainsPredicate {
                min_count: Some(min),
                max_count: None,
                predicate: Some(Box::new(w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Object(
                        w::ObjectPredicate {
                            fields: [
                                ("name".into(), eq(text(name))),
                                (
                                    "wear".into(),
                                    eq(w::AttributeValue {
                                        value: Some(w::attribute_value::Value::Decimal(
                                            wear.into(),
                                        )),
                                    }),
                                ),
                            ]
                            .into(),
                        },
                    )),
                })),
            },
        ))),
    }
}
#[test]
fn history_leaf_indexes_preserve_duplicate_counts_and_same_object_correlation() {
    let (_dir, replica) = setup();
    let mut row = order("stickers", "1", Some(10));
    row.attributes.insert(
        "stickers".into(),
        eq(w::AttributeValue {
            value: Some(w::attribute_value::Value::List(w::AttributeValues {
                values: vec![
                    sticker("gold", "0.1"),
                    sticker("gold", "0.1"),
                    sticker("silver", "0.2"),
                ],
            })),
        }),
    );
    apply(&replica, vec![upsert(entity(E::Order(row)))], "c1");
    let mut count_query = read(0, true);
    if let Some(w::selection::Source::Scope(scope)) =
        &mut count_query.selection.as_mut().unwrap().source
    {
        scope.attributes.insert(
            "stickers".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Contains(Box::new(
                    w::ContainsPredicate {
                        predicate: Some(Box::new(w::AttributePredicate {
                            predicate: Some(w::attribute_predicate::Predicate::Object(
                                w::ObjectPredicate::default(),
                            )),
                        })),
                        min_count: Some(3),
                        max_count: Some(3),
                    },
                ))),
            },
        );
    }
    assert_eq!(
        replica.read(count_query).unwrap().page.entities.len(),
        1,
        "empty object predicates can count every object occurrence"
    );
    for (name, wear, min, expected) in [
        ("gold", "0.10", 2, 1),
        ("gold", "0.1", 3, 0),
        ("gold", "0.2", 1, 0),
        ("missing", "0.1", 0, 1),
    ] {
        let mut query = read(0, true);
        let scope = match query.selection.as_mut().unwrap().source.as_mut().unwrap() {
            w::selection::Source::Scope(s) => s,
            _ => unreachable!(),
        };
        scope
            .attributes
            .insert("stickers".into(), contains(name, wear, min));
        assert_eq!(replica.read(query).unwrap().page.entities.len(), expected);
    }
}

#[test]
fn book_directory_tracks_feed_references_and_price_mode_currency_moves_without_ghosts() {
    let (_dir, replica) = setup();
    let feed = entity(E::Feed(w::Feed {
        id: "feed".into(),
        books: vec![order("a", "1", None).book.unwrap()],
        ..Default::default()
    }));
    apply(
        &replica,
        vec![
            upsert(feed),
            upsert(entity(E::Order(order("a", "1", None)))),
        ],
        "c1",
    );
    assert_eq!(replica.books(None, 10).unwrap().books.len(), 1);
    let mut moved = order("a", "2", None);
    moved.mode = 2;
    moved.book.as_mut().unwrap().currency_id = "XTR".into();
    apply(&replica, vec![upsert(entity(E::Order(moved)))], "c2");
    assert!(
        replica
            .read(read(1, false))
            .unwrap()
            .page
            .entities
            .is_empty()
    );
    assert_eq!(replica.books(None, 10).unwrap().books.len(), 2);
    apply(
        &replica,
        vec![w::Change {
            change: Some(w::change::Change::Remove(w::Removal {
                key: Some(w::Key {
                    kind: 6,
                    id: "feed".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }],
        "c3",
    );
    assert_eq!(replica.books(None, 10).unwrap().books[0].currency_id, "XTR");
    apply(
        &replica,
        vec![w::Change {
            change: Some(w::change::Change::Remove(w::Removal {
                key: Some(w::Key {
                    kind: 7,
                    venue_id: "venue".into(),
                    id: "a".into(),
                }),
                ..Default::default()
            })),
        }],
        "c4",
    );
    assert!(replica.books(None, 10).unwrap().books.is_empty());
}

#[test]
fn shared_book_references_and_directory_pagination_survive_restart() {
    let (directory, replica) = setup();
    let primary = order("a", "1", None).book.unwrap();
    let mut declared_only = primary.clone();
    declared_only.venue_id = "zeta".into();
    let mut bid = order("b", "2", None);
    bid.mode = w::OrderMode::Bid as i32;
    let mut stars = order("stars", "3", None);
    stars.book.as_mut().unwrap().currency_id = "XTR".into();
    apply(
        &replica,
        vec![
            upsert(entity(E::Feed(w::Feed {
                id: "first".into(),
                books: vec![primary.clone(), primary.clone(), declared_only.clone()],
                ..Default::default()
            }))),
            upsert(entity(E::Feed(w::Feed {
                id: "second".into(),
                books: vec![primary.clone()],
                ..Default::default()
            }))),
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(bid))),
            upsert(entity(E::Order(stars))),
        ],
        "books1",
    );
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let page = replica.books(after, 1).unwrap();
        all.extend(page.books);
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(
        all,
        [
            primary.clone(),
            w::BookKey {
                currency_id: "XTR".into(),
                ..primary.clone()
            },
            declared_only
        ]
    );
    let remove = |kind, id: &str| w::Change {
        change: Some(w::change::Change::Remove(w::Removal {
            key: Some(w::Key {
                kind,
                id: id.into(),
                venue_id: if kind == 7 {
                    "venue".into()
                } else {
                    String::new()
                },
            }),
            ..Default::default()
        })),
    };
    apply(
        &replica,
        vec![remove(6, "first"), remove(7, "a"), remove(6, "second")],
        "books2",
    );
    assert_eq!(
        replica.books(None, 10).unwrap().books.len(),
        2,
        "the remaining bid keeps its book alive"
    );
    apply(&replica, vec![remove(7, "b")], "books3");
    let expected = replica.books(None, 10).unwrap();
    assert_eq!(expected.books.len(), 1);
    assert_eq!(expected.books[0].currency_id, "XTR");
    drop(replica);
    let restored = Fixture::open(directory.path(), "test-source").unwrap();
    assert_eq!(restored.books(None, 10).unwrap(), expected);
}

#[test]
fn legacy_continuations_keep_their_shape_and_exact_boundaries() {
    use crate::types::{EntityKind, Key, Position, Sort};
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::json;
    assert_eq!(
        serde_json::to_value(Key::order("venue".into(), "a".into())).unwrap(),
        json!([7, "venue", "a"])
    );
    let position = Position {
        executed_at_ms: 10,
        venue_id: "venue".into(),
        order_id: "x".into(),
    };
    assert_eq!(
        serde_json::to_value(position).unwrap(),
        json!([10, "venue", "x"])
    );
    assert_eq!(serde_json::to_value(Sort::PriceDesc).unwrap(), json!(2));
    assert!(serde_json::from_value::<Key>(json!([0, "venue", "a"])).is_err());
    assert!(serde_json::from_value::<Key>(json!([1, "venue", "a"])).is_err());
    assert!(serde_json::from_value::<Sort>(json!(99)).is_err());
    let (_directory, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("b", "1", None)))),
            upsert(entity(E::Order(order("c", "2", None)))),
            upsert(entity(E::Order(order("x", "1", Some(10))))),
            upsert(entity(E::Order(order("y", "1", Some(20))))),
        ],
        "typed",
    );
    for (history, sort, boundary, expected) in [
        (false, 1, json!({"Price":["1",[7,"venue","a"]]}), "b"),
        (false, 2, json!({"Price":["2",[7,"venue","c"]]}), "b"),
        (false, 0, json!({"Current":[7,"venue","a"]}), "b"),
        (true, 0, json!({"History":[10,"venue","x"]}), "y"),
    ] {
        let mut query = read(1, history);
        query.sort = sort;
        let filter = crate::query::Filter::compile(&replica.view().unwrap().state, &query).unwrap();
        query.page_token = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "selection":filter.fingerprint, "sort":sort, "position":boundary, "snapshot":null
            }))
            .unwrap(),
        );
        assert_eq!(ids(&replica.read(query).unwrap().page), [expected]);
    }
    // Inclusive price bounds must retain every identity tied at the same exact price.
    let mut query = read(1, false);
    query.limit = 10;
    if let Some(w::selection::Source::Scope(scope)) =
        query.selection.as_mut().unwrap().source.as_mut()
    {
        scope.price = Some(w::PriceRange {
            min: Some("1".into()),
            max: Some("1.000".into()),
        });
    }
    assert_eq!(ids(&replica.read(query.clone()).unwrap().page), ["a", "b"]);
    query.sort = 2;
    assert_eq!(ids(&replica.read(query).unwrap().page), ["b", "a"]);
    let mut invalid = read(1, false);
    invalid.selection.as_mut().unwrap().kinds = vec![0];
    assert!(replica.read(invalid).is_err());
    let mut keys = replica
        .view()
        .unwrap()
        .state
        .entities
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    assert_eq!(keys.first().unwrap().kind, EntityKind::Venue);
}

#[test]
fn time_number_and_price_sort_whole_books_and_history_with_stable_keyset_pages() {
    let (dir, replica) = setup();
    let numbered = |id: &str, price: &str, time: i64, number: i64, history: bool| {
        let mut order = order(id, price, history.then_some(time));
        order.observed_at_ms = if history { 100 - time } else { time };
        order.attributes.insert(
            "tg.number".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::Integer(number)),
                })),
            },
        );
        order
    };
    let mut changes = Vec::new();
    for history in [false, true] {
        for (id, price, time, number) in [
            ("a", "10000000000000000.000000002", 30, 10),
            ("b", "10000000000000000.000000001", 10, 2),
            ("c", "2", 20, 5),
            ("d", "2", 20, 5),
        ] {
            changes.push(upsert(entity(E::Order(numbered(
                &format!("{}-{id}", if history { "hist" } else { "act" }),
                price,
                time,
                number,
                history,
            )))));
        }
    }
    apply(&replica, changes, "ranked1");
    let collect = |replica: &Fixture, sort: i32, history: bool| {
        let mut query = read(0, history);
        query.sort = sort;
        query.limit = 2;
        let mut result = Vec::new();
        loop {
            let page = replica.read(query.clone()).unwrap().page;
            result.extend(ids(&page));
            if page.next_page_token.is_empty() {
                break;
            }
            query.page_token = page.next_page_token;
        }
        result
    };
    for history in [false, true] {
        for (sort, expected) in [
            (1, vec!["c", "d", "b", "a"]),
            (2, vec!["a", "b", "d", "c"]),
            (3, vec!["b", "c", "d", "a"]),
            (4, vec!["a", "d", "c", "b"]),
            (5, vec!["b", "c", "d", "a"]),
            (6, vec!["a", "d", "c", "b"]),
        ] {
            let expected: Vec<_> = expected
                .into_iter()
                .map(|id| format!("{}-{id}", if history { "hist" } else { "act" }))
                .collect();
            assert_eq!(
                collect(&replica, sort, history),
                expected,
                "sort {sort}, history {history}"
            );
        }
    }
    let mut snapshot = read(0, false);
    snapshot.sort = 4;
    snapshot.limit = 2;
    let first = replica.read(snapshot.clone()).unwrap().page;
    assert_eq!(ids(&first), ["act-a", "act-d"]);
    apply(
        &replica,
        vec![upsert(entity(E::Order(numbered(
            "act-a", "3", 0, 99, false,
        ))))],
        "ranked2",
    );
    snapshot.page_token = first.next_page_token;
    assert_eq!(
        ids(&replica.read(snapshot).unwrap().page),
        ["act-c", "act-b"]
    );
    assert_eq!(
        collect(&replica, 4, false),
        ["act-d", "act-c", "act-b", "act-a"]
    );
    drop(replica);
    let reopened = Fixture::open(dir.path(), "test-source").unwrap();
    assert_eq!(
        collect(&reopened, 6, false),
        ["act-a", "act-d", "act-c", "act-b"]
    );
    assert_eq!(
        collect(&reopened, 4, true),
        ["hist-a", "hist-d", "hist-c", "hist-b"]
    );
}

#[test]
fn expired_views_release_old_roots_without_client_reads() {
    let (_dir, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("b", "2", None)))),
        ],
        "c1",
    );
    let first = replica
        .read(read(w::ReadMode::Snapshot as i32, false))
        .unwrap();
    assert!(!first.page.next_page_token.is_empty());
    let weak = {
        let mut views = replica.reader.0.views.lock().unwrap();
        let retained = views.values_mut().next().unwrap();
        retained.until = Instant::now() - Duration::from_secs(1);
        Arc::downgrade(&retained.view)
    };
    apply(&replica, vec![], "c2"); // no subsequent API read is needed
    assert!(replica.reader.0.views.lock().unwrap().is_empty());
    assert!(weak.upgrade().is_none());
    // Idle maintenance uses the same cleanup, including without a new commit.
    replica
        .read(read(w::ReadMode::Snapshot as i32, false))
        .unwrap();
    replica
        .reader
        .0
        .views
        .lock()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()
        .until = Instant::now() - Duration::from_secs(1);
    replica.writer.lock().unwrap().expire_views();
    assert!(replica.reader.0.views.lock().unwrap().is_empty());
}

#[test]
fn scoped_revisions_survive_coalescing_moves_deletions_and_replacements() {
    let (_dir, replica) = setup();
    let mut other = order("b", "2", None);
    other.book.as_mut().unwrap().currency_id = "XTR".into();
    let a = order("a", "1", None);
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(a.clone()))),
            upsert(entity(E::Order(other.clone()))),
        ],
        "c1",
    );
    let request = read(w::ReadMode::Live as i32, false);
    let revision = replica.reader.read_revision(&request).unwrap();
    let catalog = replica.reader.catalog_revision().unwrap();
    other.price = "3".into();
    apply(&replica, vec![upsert(entity(E::Order(other)))], "c2");
    assert_eq!(replica.reader.read_revision(&request).unwrap(), revision);
    assert_eq!(replica.reader.catalog_revision().unwrap(), catalog);
    let mut changed = a.clone();
    changed.price = "4".into();
    apply(
        &replica,
        vec![upsert(entity(E::Order(changed.clone())))],
        "c3",
    );
    apply(&replica, vec![], "c4"); // coalesced notification must not hide c3
    assert_ne!(replica.reader.read_revision(&request).unwrap(), revision);
    assert_eq!(replica.reader.catalog_revision().unwrap(), catalog);
    let current = replica.reader.read_revision(&request).unwrap();
    apply(
        &replica,
        vec![upsert(entity(E::Order(changed.clone())))],
        "c5",
    );
    assert_eq!(replica.reader.read_revision(&request).unwrap(), current);
    changed.book.as_mut().unwrap().currency_id = "XTR".into();
    apply(&replica, vec![upsert(entity(E::Order(changed)))], "c6");
    assert_ne!(replica.reader.read_revision(&request).unwrap(), current);
    assert_ne!(replica.reader.catalog_revision().unwrap(), catalog);
    apply(&replica, vec![upsert(entity(E::Order(a)))], "c7");
    let before = replica.reader.read_revision(&request).unwrap();
    let store = replica.writer.lock().unwrap().store.clone();
    replica.install(store).unwrap();
    assert_ne!(replica.reader.read_revision(&request).unwrap(), before);
}

#[tokio::test]
async fn maintenance_releases_expired_snapshots_during_idle_upstream() {
    let (_dir, replica) = setup();
    apply(
        &replica,
        vec![
            upsert(entity(E::Order(order("a", "1", None)))),
            upsert(entity(E::Order(order("b", "2", None)))),
        ],
        "c1",
    );
    replica
        .read(read(w::ReadMode::Snapshot as i32, false))
        .unwrap();
    replica
        .reader
        .0
        .views
        .lock()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()
        .until = Instant::now() - Duration::from_secs(1);
    // A loopback listener accepts no RPCs; upstream activity cannot do the cleanup.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = market_hub_client::HubClient::new(
        format!("http://{}", listener.local_addr().unwrap()),
        "idle.synthetic-test-signature".into(),
    )
    .unwrap();
    let writer = replica.writer.into_inner().unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = crate::ReplicationWorker {
        writer: Arc::new(Mutex::new(writer)),
    };
    let task = tokio::spawn(worker.run(hub, stop.clone()));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if replica.reader.0.views.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    stop.cancel();
    task.await.unwrap().unwrap();
}
