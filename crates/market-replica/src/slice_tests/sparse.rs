use super::*;
fn model(id: &str) -> w::AttributePredicate {
    w::AttributePredicate {
        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
            value: Some(w::attribute_value::Value::OptionId(id.into())),
        })),
    }
}
fn listing(id: &str, price: usize, kind: &str) -> w::Order {
    let mut row = order(id, &price.to_string(), None);
    row.observed_at_ms = price as i64;
    row.attributes.insert("model".into(), model(kind));
    row.attributes.insert(
        "tg.number".into(),
        w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                value: Some(w::attribute_value::Value::Integer(price as i64)),
            })),
        },
    );
    row
}
fn request(sort: i32, selected: w::AttributePredicate) -> w::ReadRequest {
    let mut request = read(1, false);
    request.sort = sort;
    let Some(w::selection::Source::Scope(scope)) = &mut request.selection.as_mut().unwrap().source
    else {
        unreachable!()
    };
    scope.attributes.insert("model".into(), selected);
    request
}
fn complete(reader: &ReplicaReader, request: w::ReadRequest) -> ReadBatch {
    reader
        .query_range_checked(
            request,
            RangeRequest {
                range: KeyRange {
                    start: None,
                    end: None,
                },
                limit: 100,
                snapshot: None,
                options: Default::default(),
            },
            true,
            |_, _| Ok(()),
            |_| Ok(true),
        )
        .unwrap()
}
#[test]
fn sparse_ranked_filters_complete_page_and_refresh_without_scanning_unrelated_orders() {
    let (dir, mut writer) = setup();
    let mut changes = vec![
        upsert(entity(E::Attribute(w::Attribute {
            id: "model".into(),
            name: "Model".into(),
            value_type: 1,
            ..Default::default()
        }))),
        upsert(entity(E::ItemType(w::ItemType {
            id: "gift".into(),
            name: "Gift".into(),
            quantity_step: "1".into(),
            attributes: [
                ("model".into(), Default::default()),
                ("tg.number".into(), Default::default()),
            ]
            .into(),
            ..Default::default()
        }))),
    ];
    for id in ["rare", "common", "absent"] {
        changes.push(upsert(entity(E::AttributeOption(w::AttributeOption {
            id: id.into(),
            attribute_id: "model".into(),
            name: id.into(),
            ..Default::default()
        }))));
    }
    for i in 0..READ_LIMITS.max_scan_candidates + 2000 {
        changes.push(upsert(entity(E::Order(listing(
            &format!("common-{i}"),
            i + 1,
            "common",
        )))));
    }
    for i in 0..62 {
        changes.push(upsert(entity(E::Order(listing(
            &format!("rare-{i}"),
            (i + 1) * 200,
            "rare",
        )))));
    }
    writer.commit(changes, "sparse".into()).unwrap();
    let reader = writer.reader();
    let pinned = reader.snapshot().unwrap();
    for sort in 1..=6 {
        let query = request(sort, model("rare"));
        let initial = reader
            .query_range_checked(
                query.clone(),
                RangeRequest {
                    range: KeyRange {
                        start: None,
                        end: None,
                    },
                    limit: 80,
                    snapshot: None,
                    options: Default::default(),
                },
                false,
                |_, _| Ok(()),
                |_| Ok(true),
            )
            .unwrap();
        assert_eq!(
            initial.entries.len(),
            62,
            "{sort}: the first read reaches the true end"
        );
        assert!(initial.range.end.is_none());
        assert_eq!(complete(&reader, query.clone()).entries.len(), 62);
        let mut start = None;
        let mut seen = Vec::new();
        loop {
            let page = reader
                .query_range_checked(
                    query.clone(),
                    RangeRequest {
                        range: KeyRange {
                            start: start.clone(),
                            end: None,
                        },
                        limit: 25,
                        snapshot: None,
                        options: Default::default(),
                    },
                    false,
                    |_, _| Ok(()),
                    |_| Ok(true),
                )
                .unwrap();
            seen.extend(ids(&page));
            let Some(end) = page.range.end else {
                break;
            };
            start = Some(end);
        }
        assert_eq!(
            seen,
            ids(&initial),
            "{sort}: half-open pagination retains exact ordering"
        );
        assert!(
            complete(&reader, request(sort, model("absent")))
                .entries
                .is_empty()
        );
        let union = w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::OneOf(
                w::AttributeValues {
                    values: vec![
                        w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId("rare".into())),
                        },
                        w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId("absent".into())),
                        },
                    ],
                },
            )),
        };
        assert_eq!(ids(&complete(&reader, request(sort, union))), seen);
    }
    // Repeated unrelated market updates must not make a complete live range exceed its scan budget.
    for i in 0..3 {
        writer
            .commit(
                vec![upsert(entity(E::Order(listing(
                    "unrelated-update",
                    5000 + i,
                    "common",
                ))))],
                format!("tick-{i}"),
            )
            .unwrap();
        assert_eq!(
            complete(&reader, request(1, model("rare"))).entries.len(),
            62
        );
    }
    writer
        .commit(
            vec![
                upsert(entity(E::Order(listing("common-0", 1, "rare")))),
                upsert(entity(E::Order(listing("rare-0", 50000, "rare")))),
                w::Change {
                    change: Some(w::change::Change::Remove(w::Removal {
                        key: Some(w::Key {
                            kind: 7,
                            venue_id: "venue".into(),
                            id: "rare-1".into(),
                        }),
                        ..Default::default()
                    })),
                },
            ],
            "replace-and-remove".into(),
        )
        .unwrap();
    let current = complete(&reader, request(1, model("rare")));
    assert_eq!(current.entries.len(), 62);
    assert_eq!(ids(&current).first().map(String::as_str), Some("common-0"));
    assert_eq!(ids(&current).last().map(String::as_str), Some("rare-0"));
    assert_eq!(
        ids(&complete(&pinned, request(1, model("rare"))))
            .first()
            .map(String::as_str),
        Some("rare-0")
    );
    drop(reader);
    drop(pinned);
    drop(writer);
    let reopened = ReplicaWriter::open(dir.path(), "test").unwrap();
    assert_eq!(
        ids(&complete(&reopened.reader(), request(1, model("rare")))),
        ids(&current)
    );
}
