use super::*;
use w::entity::Entity as E;
fn entity(value: E) -> w::Entity {
    w::Entity {
        entity: Some(value),
    }
}
fn upsert(value: w::Entity) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Upsert(value)),
    }
}
fn catalog() -> Vec<w::Change> {
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
fn order(id: &str, price: &str, time: Option<i64>) -> w::Order {
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
fn read(mode: i32, history: bool) -> w::ReadRequest {
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
fn setup() -> (tempfile::TempDir, Replica) {
    let dir = tempfile::tempdir().unwrap();
    let replica = Replica::open(dir.path(), "test-source").unwrap();
    let store = replica.0.store.lock().unwrap().clone();
    store.apply(&catalog(), Some("c0")).unwrap();
    replica.install(store).unwrap();
    (dir, replica)
}
fn apply(replica: &Replica, changes: Vec<w::Change>, cursor: &str) {
    replica
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
    let reopened = Replica::open(dir.path(), "test-source").unwrap();
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
    let restored = Replica::open(dir.path(), "test-source").unwrap();
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
#[tokio::test]
async fn snapshot_paging_spools_changes_before_older_snapshot_rows_and_resumes_saved_state() {
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response, transport::Server};
    #[derive(Clone)]
    struct Peer {
        reads: Arc<std::sync::atomic::AtomicUsize>,
        watches: Arc<std::sync::atomic::AtomicUsize>,
        reset: Arc<std::sync::atomic::AtomicBool>,
    }
    #[tonic::async_trait]
    impl w::market_read_server::MarketRead for Peer {
        async fn books(
            &self,
            _: Request<w::BooksRequest>,
        ) -> Result<Response<w::BooksPage>, Status> {
            Err(Status::unimplemented("replication never needs Books"))
        }
        type ReadStream = tokio_stream::Once<Result<w::Page, Status>>;
        async fn read(
            &self,
            r: Request<w::ReadRequest>,
        ) -> Result<Response<Self::ReadStream>, Status> {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert!(r.get_ref().selection.is_none());
            if self.reset.load(std::sync::atomic::Ordering::SeqCst) {
                let mut entities = catalog()
                    .into_iter()
                    .map(|change| match change.change.unwrap() {
                        w::change::Change::Upsert(e) => e,
                        _ => unreachable!(),
                    })
                    .collect::<Vec<_>>();
                entities.push(entity(E::Order(order("new", "5", None))));
                return Ok(Response::new(tokio_stream::once(Ok(w::Page {
                    entities,
                    cursor: "new0".into(),
                    ..Default::default()
                }))));
            }
            let page = if r.get_ref().page_token.is_empty() {
                w::Page {
                    entities: catalog()
                        .into_iter()
                        .map(|c| match c.change.unwrap() {
                            w::change::Change::Upsert(e) => e,
                            _ => unreachable!(),
                        })
                        .collect(),
                    next_page_token: "later".into(),
                    cursor: "c0".into(),
                }
            } else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                w::Page {
                    entities: vec![entity(E::Order(order("late-row", "9", None)))],
                    cursor: "c0".into(),
                    ..Default::default()
                }
            };
            Ok(Response::new(tokio_stream::once(Ok(page))))
        }
        type WatchStream =
            std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<w::Batch, Status>> + Send>>;
        async fn watch(
            &self,
            r: Request<w::WatchRequest>,
        ) -> Result<Response<Self::WatchStream>, Status> {
            use tokio_stream::StreamExt;
            self.watches
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let cursor = r.into_inner().after_cursor;
            if self.reset.load(std::sync::atomic::Ordering::SeqCst) && cursor != "new0" {
                return Err(Status::failed_precondition("old generation"));
            }
            let frames = if cursor == "new0" {
                vec![Ok(w::Batch {
                    cursor,
                    changes: vec![],
                })]
            } else if cursor == "c0" {
                vec![
                    Ok(w::Batch {
                        cursor: "c0".into(),
                        changes: vec![],
                    }),
                    Ok(w::Batch {
                        cursor: "c1".into(),
                        changes: vec![
                            upsert(entity(E::Order(order("late-row", "1", None)))),
                            upsert(entity(E::Order(order("executed", "2", Some(1))))),
                        ],
                    }),
                ]
            } else {
                assert_eq!(cursor, "c1");
                vec![Ok(w::Batch {
                    cursor,
                    changes: vec![],
                })]
            };
            Ok(Response::new(Box::pin(
                tokio_stream::iter(frames).chain(tokio_stream::pending()),
            )))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let channel = tonic::transport::Endpoint::from_shared(format!(
        "http://{}",
        listener.local_addr().unwrap()
    ))
    .unwrap()
    .connect_lazy();
    let peer = Peer {
        reads: Arc::default(),
        watches: Arc::default(),
        reset: Arc::default(),
    };
    let server = peer.clone();
    let stopped = tokio_util::sync::CancellationToken::new();
    let cancel = stopped.clone();
    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(w::market_read_server::MarketReadServer::new(server))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(listener),
                cancel.cancelled_owned(),
            )
            .await
            .unwrap();
    });
    for phase in 0..3 {
        let restored = phase > 0;
        if phase == 2 {
            peer.reset.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let replica = Replica::open(dir.path(), "test-source").unwrap();
        if restored {
            assert!(replica.status().ready);
        }
        let target = replica.clone();
        let cancel = tokio_util::sync::CancellationToken::new();
        let worker_stop = cancel.clone();
        let channel = channel.clone();
        let worker = tokio::spawn(async move {
            target
                .run(channel, "Bearer test.token".parse().unwrap(), worker_stop)
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while replica.status().state != SyncState::Live
                || replica
                    .view()
                    .map(|v| v.cursor != if phase == 2 { "new0" } else { "c1" })
                    .unwrap_or(true)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let current = replica.read(read(1, false)).unwrap();
        assert_eq!(
            state::order(&current.page.entities[0]).unwrap().price,
            if phase == 2 { "5" } else { "1" }
        );
        if phase == 2 {
            assert!(
                replica
                    .read(read(0, true))
                    .unwrap()
                    .page
                    .entities
                    .is_empty(),
                "replacement snapshots remove old history"
            );
        } else {
            assert_eq!(
                ids(&replica.read(read(0, true)).unwrap().page),
                ["executed"]
            );
        }
        cancel.cancel();
        worker.await.unwrap();
        drop(replica);
    }
    assert_eq!(
        peer.reads.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "restart must resume without a new snapshot"
    );
    assert_eq!(peer.watches.load(std::sync::atomic::Ordering::SeqCst), 4);
    stopped.cancel();
    server.await.unwrap();
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
