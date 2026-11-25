use super::*;
use crate::{
    MarketDataService,
    tests::{fixture, order},
};
use market_gifts_hub::input::{GiftFilters, GiftOrderQuery};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn query(collection: &str) -> GiftOrderQuery {
    GiftOrderQuery::new(GiftFilters::new(
        market_gifts_hub::input::CollectionRef::new(collection).unwrap(),
        "GRAM".into(),
    ))
}
async fn terminal(updates: &mut watch::Receiver<DataUpdate>) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(*updates.borrow_and_update(), DataUpdate::Error(_)) {
                return;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
async fn price(updates: &mut watch::Receiver<DataUpdate>, expected: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let DataUpdate::Orders { result, .. } = updates.borrow_and_update().clone()
                && result
                    .data
                    .first()
                    .is_some_and(|row| row.order.price.as_str() == expected)
            {
                return;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
struct Harness {
    service: MarketDataService,
    reads: market_replica::ReadRuntime,
}
impl Harness {
    async fn new(reader: market_replica::ReplicaReader, queries: usize) -> Self {
        let reads =
            market_replica::ReadRuntime::new(reader, queries, CancellationToken::new()).unwrap();
        let service = MarketDataService::start(reads.handle(), 1).await.unwrap();
        Self { service, reads }
    }
    async fn shutdown(self) {
        let stop = CancellationToken::new();
        stop.cancel();
        self.service.task.run(stop).await.unwrap();
        self.reads.shutdown().await;
    }
}

#[tokio::test]
async fn shared_leases_and_clones_cancel_only_after_the_final_lease_is_dropped() {
    let (_directory, writer) = fixture();
    let h = Harness::new(writer.reader(), 2).await;
    let (a, mut first) = h
        .service
        .handle
        .watch_orders(query("gift"))
        .await
        .unwrap()
        .test_parts();
    let clone = a.clone();
    let (b, mut second) = h
        .service
        .handle
        .watch_orders(query("gift"))
        .await
        .unwrap()
        .test_parts();
    assert!(first.same_channel(&second));
    let stopped = a._feed.stop.clone();
    price(&mut first, "3").await;
    drop(a);
    drop(b);
    assert!(
        !stopped.is_cancelled(),
        "a cloned lease still owns the shared producer"
    );
    drop(clone);
    assert!(
        stopped.is_cancelled(),
        "cleanup must not wait for an actor mailbox message"
    );
    terminal(&mut first).await;
    terminal(&mut second).await;
    h.shutdown().await;
}

#[tokio::test]
async fn failed_initializations_release_capacity_before_retries() {
    let (_directory, writer) = fixture();
    let h = Harness::new(writer.reader(), 2).await;
    for _ in 0..16 {
        assert!(
            h.service
                .handle
                .watch_orders(query("missing"))
                .await
                .is_err()
        );
    }
    let (lease, mut updates) = h
        .service
        .handle
        .watch_orders(query("gift"))
        .await
        .unwrap()
        .test_parts();
    price(&mut updates, "3").await;
    drop(lease);
    terminal(&mut updates).await;
    h.shutdown().await;
}

#[tokio::test]
async fn losing_all_receivers_stops_a_feed_and_old_leases_cannot_cancel_its_replacement() {
    let (_directory, mut writer) = fixture();
    let h = Harness::new(writer.reader(), 2).await;
    let (old, mut updates) = h
        .service
        .handle
        .watch_orders(query("gift"))
        .await
        .unwrap()
        .test_parts();
    price(&mut updates, "3").await;
    drop(updates);
    tokio::time::timeout(Duration::from_secs(2), old._feed.stop.cancelled())
        .await
        .unwrap();
    // Observe completion even though the caller retained only its lease.
    terminal(&mut old._feed.sender.subscribe()).await;
    let (new, mut updates) = h
        .service
        .handle
        .watch_orders(query("gift"))
        .await
        .unwrap()
        .test_parts();
    assert!(!Arc::ptr_eq(&old._feed, &new._feed));
    drop(old);
    writer
        .apply(market_hub_protocol::wire::Batch {
            cursor: "replacement".into(),
            changes: vec![order("2")],
        })
        .unwrap();
    price(&mut updates, "2").await;
    assert!(!new._feed.stop.is_cancelled());
    drop(new);
    terminal(&mut updates).await;
    h.shutdown().await;
}

#[tokio::test]
async fn cancelling_a_seed_waiting_for_capacity_releases_its_initialization_slot() {
    let (_directory, writer) = fixture();
    let h = Harness::new(writer.reader(), 1).await;
    let busy = h.reads.handle().budget().try_query().unwrap();
    let handle = h.service.handle.clone();
    let mut pending = tokio::spawn(async move { handle.watch_orders(query("gift")).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut pending)
            .await
            .is_err()
    );
    pending.abort();
    assert!(matches!(pending.await, Err(error) if error.is_cancelled()));
    let handle = h.service.handle.clone();
    let mut replacement = tokio::spawn(async move { handle.watch_orders(query("gift")).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut replacement)
            .await
            .is_err()
    );
    drop(busy);
    let (lease, mut updates) = replacement.await.unwrap().unwrap().test_parts();
    price(&mut updates, "3").await;
    drop(lease);
    terminal(&mut updates).await;
    h.shutdown().await;
}

#[tokio::test]
async fn equivalent_catalog_watches_share_a_feed_and_keep_independent_leases() {
    let (_directory, mut writer) = fixture();
    let h = Harness::new(writer.reader(), 2).await;
    let (a, mut first) = h
        .service
        .handle
        .subscribe_catalog(vec!["gift".into(), "gift".into()])
        .await
        .unwrap()
        .test_parts();
    let (b, second) = h
        .service
        .handle
        .subscribe_catalog(vec!["gift".into()])
        .await
        .unwrap()
        .test_parts();
    assert!(first.same_channel(&second));
    async fn catalog(
        updates: &mut watch::Receiver<DataUpdate>,
    ) -> market_gifts_hub::input::CatalogVersions {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let DataUpdate::Catalog(value) = updates.borrow_and_update().clone() {
                    return value;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap()
    }
    let before = catalog(&mut first).await;
    drop(a);
    writer
        .apply(market_hub_protocol::wire::Batch {
            cursor: "catalog".into(),
            changes: vec![crate::tests::entity(
                market_hub_protocol::wire::entity::Entity::Currency(
                    market_hub_protocol::wire::Currency {
                        id: "GRAM".into(),
                        name: "Updated currency".into(),
                        decimals: 9,
                    },
                ),
            )],
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            first.changed().await.unwrap();
            if let DataUpdate::Catalog(value) = first.borrow_and_update().clone()
                && value != before
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    drop(b);
    terminal(&mut first).await;
    h.shutdown().await;
}

fn seed(writer: &market_replica::ReplicaWriter, params: GiftOrderQuery) -> types::OrdersFeed {
    let (request, read) = crate::query::compile(params).unwrap();
    let options = read.options;
    let batch = writer
        .reader()
        .query_range_checked(
            request.pin_request(),
            read,
            false,
            crate::query::validate_order_collection,
            crate::protocol::include_order,
        )
        .unwrap();
    let snapshot = crate::protocol::projected_read(batch).unwrap();
    types::OrdersFeed {
        request,
        options,
        initial: Arc::new(snapshot.read.result),
        version: snapshot.version,
        observed: snapshot.observed,
    }
}
async fn register(
    h: &Harness,
    feed: types::OrdersFeed,
    cancel: CancellationToken,
) -> Result<DataWatch, tonic::Status> {
    h.service
        .handle
        .subscriptions
        .send(messages::RegisterOrders { feed, cancel })
        .await
        .unwrap()
}
fn initial_price(update: &DataUpdate) -> &str {
    let DataUpdate::Orders { result, .. } = update else {
        panic!("expected orders")
    };
    result.data[0].order.price.as_str()
}

#[tokio::test]
async fn exact_current_coverage_joins_without_read_admission_or_a_second_live_slot() {
    let (_directory, writer) = fixture();
    let h = Harness::new(writer.reader(), 1).await;
    let first = h.service.handle.watch_orders(query("gift")).await.unwrap();
    let busy = h.reads.handle().budget().try_query().unwrap();
    let mut smaller = query("gift");
    smaller.limit = 1;
    let joined = tokio::time::timeout(
        Duration::from_millis(100),
        h.service.handle.watch_orders(smaller),
    )
    .await
    .expect("An exact current feed does not need another read")
    .unwrap();
    assert!(first.updates.same_channel(&joined.updates));
    assert_eq!(initial_price(&joined.initial), "3");
    drop((first, joined, busy));
    h.shutdown().await;
}

#[tokio::test]
async fn a_fresh_seed_updates_a_lagging_feed_instead_of_exhausting_capacity() {
    let (_directory, mut writer) = fixture();
    let h = Harness::new(writer.reader(), 1).await;
    let first = h.service.handle.watch_orders(query("gift")).await.unwrap();
    let busy = h.reads.handle().budget().try_query().unwrap();
    writer
        .apply(market_hub_protocol::wire::Batch {
            cursor: "new".into(),
            changes: vec![order("2")],
        })
        .unwrap();
    let fresh = seed(&writer, query("gift"));
    let joined = register(&h, fresh, CancellationToken::new()).await.unwrap();
    assert!(first.updates.same_channel(&joined.updates));
    assert_eq!(initial_price(&joined.initial), "2");
    assert_eq!(initial_price(&first.updates.borrow()), "2");
    drop((busy, joined, first));
    h.shutdown().await;
}

#[tokio::test]
async fn delayed_seed_registration_and_read_completion_cannot_roll_back_shared_state() {
    let (_directory, mut writer) = fixture();
    let h = Harness::new(writer.reader(), 1).await;
    let first = h.service.handle.watch_orders(query("gift")).await.unwrap();
    let busy = h.reads.handle().budget().try_query().unwrap();
    let old = seed(&writer, query("gift"));
    writer
        .apply(market_hub_protocol::wire::Batch {
            cursor: "new".into(),
            changes: vec![order("2")],
        })
        .unwrap();
    let current = register(&h, seed(&writer, query("gift")), CancellationToken::new())
        .await
        .unwrap();
    let delayed = register(&h, old, CancellationToken::new()).await.unwrap();
    assert!(current.updates.same_channel(&delayed.updates));
    assert_eq!(
        initial_price(&delayed.initial),
        "3",
        "The caller retains its own pinned baseline"
    );
    assert_eq!(
        initial_price(&delayed.updates.borrow()),
        "2",
        "The first shared update is current, never older than the seed"
    );
    let mut updates = first.updates.clone();
    updates.borrow_and_update();
    assert!(!types::publish_orders(
        &first.lease._feed.sender,
        delayed.initial.clone()
    ));
    assert!(!updates.has_changed().unwrap());
    assert_eq!(initial_price(&updates.borrow()), "2");
    drop((busy, first, current, delayed, updates));
    h.shutdown().await;
}

#[tokio::test]
async fn an_empty_newer_book_cannot_be_replaced_by_an_older_nonempty_read() {
    use market_hub_protocol::wire as w;
    let (_directory, mut writer) = fixture();
    let h = Harness::new(writer.reader(), 1).await;
    let first = h.service.handle.watch_orders(query("gift")).await.unwrap();
    let busy = h.reads.handle().budget().try_query().unwrap();
    writer
        .apply(w::Batch {
            cursor: "empty".into(),
            changes: vec![w::Change {
                change: Some(w::change::Change::Remove(w::Removal {
                    key: Some(w::Key {
                        kind: w::EntityKind::Order as i32,
                        venue_id: "venue".into(),
                        id: "order".into(),
                    }),
                    ..Default::default()
                })),
            }],
        })
        .unwrap();
    let empty = register(&h, seed(&writer, query("gift")), CancellationToken::new())
        .await
        .unwrap();
    assert!(!types::publish_orders(
        &first.lease._feed.sender,
        first.initial.clone()
    ));
    let DataUpdate::Orders { result, .. } = empty.updates.borrow().clone() else {
        panic!("expected orders")
    };
    assert!(result.data.is_empty());
    drop((busy, first, empty));
    h.shutdown().await;
}

#[tokio::test]
async fn cancelled_registration_cannot_refresh_a_feed_or_consume_another_live_slot() {
    let (_directory, mut writer) = fixture();
    let h = Harness::new(writer.reader(), 1).await;
    let first = h.service.handle.watch_orders(query("gift")).await.unwrap();
    let busy = h.reads.handle().budget().try_query().unwrap();
    writer
        .apply(market_hub_protocol::wire::Batch {
            cursor: "new".into(),
            changes: vec![order("2")],
        })
        .unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        matches!(register(&h, seed(&writer, query("gift")), cancel).await, Err(error) if error.code() == tonic::Code::Cancelled)
    );
    assert_eq!(initial_price(&first.updates.borrow()), "3");
    let joined = register(&h, seed(&writer, query("gift")), CancellationToken::new())
        .await
        .unwrap();
    assert!(first.updates.same_channel(&joined.updates));
    drop((busy, first, joined));
    h.shutdown().await;
}

#[tokio::test]
async fn terminal_feeds_cannot_be_resurrected_by_a_late_seed() {
    let (_directory, writer) = fixture();
    let h = Harness::new(writer.reader(), 1).await;
    let watch = h.service.handle.watch_orders(query("gift")).await.unwrap();
    let initial = watch.initial.clone();
    let feed = watch.lease._feed.clone();
    feed.sender
        .send_replace(DataUpdate::Error(Status::unavailable("stopped").into()));
    assert!(!types::publish_orders(&feed.sender, initial));
    assert!(matches!(*feed.sender.borrow(), DataUpdate::Error(_)));
    drop((feed, watch));
    h.shutdown().await;
}

#[tokio::test]
async fn equal_rows_share_references_across_revisions_without_losing_dataset_identity() {
    let (_directory, mut writer) = fixture();
    let h = Harness::new(writer.reader(), 1).await;
    let first = h.service.handle.watch_orders(query("gift")).await.unwrap();
    let busy = h.reads.handle().budget().try_query().unwrap();
    let old = seed(&writer, query("gift"));
    for (cursor, price) in [("changed", "2"), ("restored", "3")] {
        writer
            .apply(market_hub_protocol::wire::Batch {
                cursor: cursor.into(),
                changes: vec![order(price)],
            })
            .unwrap();
    }
    let current = register(&h, seed(&writer, query("gift")), CancellationToken::new())
        .await
        .unwrap();
    let delayed = register(&h, old, CancellationToken::new()).await.unwrap();
    let DataUpdate::Orders {
        result: initial,
        version: initial_version,
        ..
    } = &delayed.initial
    else {
        panic!("expected orders")
    };
    let DataUpdate::Orders {
        result: published,
        version: current_version,
        ..
    } = delayed.updates.borrow().clone()
    else {
        panic!("expected orders")
    };
    assert_ne!(*initial_version, current_version);
    assert!(initial_version.same_dataset(&current_version));
    assert!(
        Arc::ptr_eq(initial, &published),
        "Equal baselines must not produce a redundant empty WebSocket delta"
    );
    drop((busy, first, current, delayed));
    h.shutdown().await;
}

#[tokio::test]
async fn the_initial_snapshot_is_preserved_when_shared_updates_grow_before_consumption() {
    use market_hub_protocol::wire as w;
    let (_directory, mut writer) = fixture();
    let named = |id: &str, price: &str| {
        let mut change = order(price);
        let Some(w::change::Change::Upsert(entity)) = &mut change.change else {
            unreachable!()
        };
        let Some(w::entity::Entity::Order(order)) = &mut entity.entity else {
            unreachable!()
        };
        order.id = id.into();
        change
    };
    writer
        .apply(w::Batch {
            cursor: "seed".into(),
            changes: vec![named("a", "1"), named("b", "2")],
        })
        .unwrap();
    let h = Harness::new(writer.reader(), 1).await;
    let mut params = query("gift");
    params.limit = 2;
    let mut watch = h.service.handle.watch_orders(params).await.unwrap();
    writer
        .apply(w::Batch {
            cursor: "arrival".into(),
            changes: vec![named("new", "0.5")],
        })
        .unwrap();
    price(&mut watch.updates, "0.5").await;
    let DataUpdate::Orders {
        result: initial, ..
    } = &watch.initial
    else {
        panic!("expected orders")
    };
    let DataUpdate::Orders {
        result: current, ..
    } = watch.updates.borrow().clone()
    else {
        panic!("expected orders")
    };
    assert_eq!(
        initial.data.len(),
        2,
        "A slow consumer still receives its caller-sized first snapshot"
    );
    assert_eq!(current.data.len(), 3);
    assert_eq!(initial.range, current.range);
    drop(watch);
    h.shutdown().await;
}

#[tokio::test]
async fn catalog_joins_leave_the_current_manifest_as_the_only_initial_data_update() {
    let (_directory, writer) = fixture();
    let h = Harness::new(writer.reader(), 1).await;
    let mut first = h
        .service
        .handle
        .subscribe_catalog(vec!["gift".into()])
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(*first.updates.borrow_and_update(), DataUpdate::Catalog(_)) {
                break;
            }
            first.updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let joined = h
        .service
        .handle
        .subscribe_catalog(vec!["gift".into()])
        .await
        .unwrap();
    assert!(matches!(joined.initial, DataUpdate::Connecting));
    assert!(matches!(*joined.updates.borrow(), DataUpdate::Catalog(_)));
    assert!(first.updates.same_channel(&joined.updates));
    drop((first, joined));
    h.shutdown().await;
}

#[tokio::test]
async fn live_history_publishes_new_executions_and_late_backfills_within_its_period() {
    use market_hub_protocol::wire as w;
    let (_directory, mut writer) = fixture();
    let h = Harness::new(writer.reader(), 2).await;
    let mut history = query("gift");
    history.source = market_gifts::OrderSource::History;
    history.period = Some(market_gifts::Period {
        from: Some("1970-01-01T00:00:01Z".parse().unwrap()),
        until: Some("1970-01-01T00:00:05Z".parse().unwrap()),
    });
    let mut watch = h.service.handle.watch_orders(history).await.unwrap();
    assert!(matches!(&watch.initial, DataUpdate::Orders { result, .. } if result.data.is_empty()));
    let execution = |id: &str, at: i64| {
        let mut change = order("2");
        if let Some(w::change::Change::Upsert(w::Entity {
            entity: Some(w::entity::Entity::Order(order)),
        })) = &mut change.change
        {
            order.id = id.into();
            order.mode = w::OrderMode::Executed as i32;
            order.executed_at_ms = Some(at);
        }
        change
    };
    writer
        .apply(w::Batch {
            cursor: "executions".into(),
            changes: vec![execution("new", 3000), execution("outside-period", 6000)],
        })
        .unwrap();
    price(&mut watch.updates, "2").await;
    assert!(
        matches!(&*watch.updates.borrow(), DataUpdate::Orders { result, .. }
        if result.data.len() == 1 && result.data[0].order.id.as_str() == "new")
    );
    writer
        .apply(w::Batch {
            cursor: "backfill".into(),
            changes: vec![execution("late", 2000), execution("newest", 4000)],
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let DataUpdate::Orders { result, .. } = watch.updates.borrow_and_update().clone()
                && result.data.len() == 3
            {
                assert_eq!(
                    result
                        .data
                        .iter()
                        .map(|row| row.order.id.as_str())
                        .collect::<Vec<_>>(),
                    ["newest", "new", "late"]
                );
                assert!(
                    result
                        .data
                        .iter()
                        .all(|row| row.order.kind == market_gifts::GiftOrderKind::Executed)
                );
                break;
            }
            watch.updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    drop(watch);
    h.shutdown().await;
}

#[tokio::test]
async fn sparse_model_watch_remains_complete_across_unrelated_market_updates() {
    use crate::tests::entity;
    use market_hub_protocol::wire as w;
    let (_directory, mut writer) = fixture();
    let mut changes = vec![
        entity(w::entity::Entity::Attribute(w::Attribute {
            id: "tg.model".into(),
            name: "Model".into(),
            value_type: 1,
            ..Default::default()
        })),
        entity(w::entity::Entity::ItemType(w::ItemType {
            id: "gift".into(),
            name: "Gift".into(),
            quantity_step: "1".into(),
            aliases: vec![w::ExternalId {
                namespace: "tg".into(),
                id: "99".into(),
            }],
            attributes: [("tg.model".into(), Default::default())].into(),
        })),
    ];
    for id in ["rare", "common"] {
        changes.push(entity(w::entity::Entity::AttributeOption(
            w::AttributeOption {
                id: id.into(),
                name: id.into(),
                attribute_id: "tg.model".into(),
                ..Default::default()
            },
        )));
    }
    let listing = |id: String, price: usize, model: &str| {
        let mut change = order(&price.to_string());
        if let Some(w::change::Change::Upsert(w::Entity {
            entity: Some(w::entity::Entity::Order(row)),
        })) = &mut change.change
        {
            row.id = id;
            row.attributes.insert(
                "tg.model".into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId(model.into())),
                    })),
                },
            );
        }
        change
    };
    for i in 0..market_replica::READ_LIMITS.max_scan_candidates + 2000 {
        changes.push(listing(format!("common-{i}"), i + 1, "common"));
    }
    for i in 0..62 {
        changes.push(listing(format!("rare-{i}"), (i + 1) * 200, "rare"));
    }
    writer
        .apply(w::Batch {
            changes,
            cursor: "sparse".into(),
        })
        .unwrap();
    let h = Harness::new(writer.reader(), 2).await;
    let mut q = query("gift");
    q.filters.models = vec!["rare".into()];
    q.limit = 80;
    let mut live = h.service.handle.watch_orders(q).await.unwrap();
    let mut clock = match &live.initial {
        DataUpdate::Orders {
            result, observed, ..
        } => {
            assert_eq!(result.data.len(), 62);
            assert!(result.range.end.is_none());
            *observed
        }
        _ => panic!("expected a complete initial model view"),
    };
    for i in 0..3 {
        writer
            .apply(w::Batch {
                changes: vec![listing("unrelated".into(), 5000 + i, "common")],
                cursor: format!("other-{i}"),
            })
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let update = live.updates.borrow_and_update().clone();
                match update {
                    DataUpdate::Error(error) => panic!("Sparse refresh failed: {error}"),
                    DataUpdate::Orders {
                        result, observed, ..
                    } if observed > clock => {
                        assert_eq!(result.data.len(), 62);
                        clock = observed;
                        break;
                    }
                    _ => {}
                }
                live.updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    writer
        .apply(w::Batch {
            changes: vec![listing("new-rare".into(), 1, "rare")],
            cursor: "new-rare".into(),
        })
        .unwrap();
    price(&mut live.updates, "1").await;
    assert!(
        matches!(&*live.updates.borrow(),DataUpdate::Orders {result,..} if result.data.len()==63)
    );
    drop(live);
    h.shutdown().await;
}
