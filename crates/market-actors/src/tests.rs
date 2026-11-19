use crate::*;
use crate::{
    catalog::CatalogActor, orderbooks::OrderbooksActor, subscriptions::MarketSubscriptionsActor,
};
use actix::prelude::*;
use market_hub_protocol::wire as w;
use market_replica::ReadResult;
use market_replica::ReplicaWriter;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tonic::Status;
pub(crate) fn entity(value: w::entity::Entity) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Upsert(w::Entity {
            entity: Some(value),
        })),
    }
}
pub(crate) fn order(price: &str) -> w::Change {
    entity(w::entity::Entity::Order(w::Order {
        id: "order".into(),
        book: Some(w::BookKey {
            venue_id: "venue".into(),
            item_type_id: "gift".into(),
            currency_id: "GRAM".into(),
        }),
        mode: 1,
        price: price.into(),
        quantity: "1".into(),
        ..Default::default()
    }))
}
fn gift_query() -> market_gifts::GiftOrdersQuery {
    market_gifts::GiftOrdersQuery::new(market_gifts::GiftFilters::new("gift".into(), "GRAM".into()))
}
fn raw_query() -> w::ReadRequest {
    w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(w::Scope {
                item_type_id: Some("gift".into()),
                currency_id: Some("GRAM".into()),
                modes: vec![1],
                ..Default::default()
            })),
            ..Default::default()
        }),
        limit: 50,
        sort: 1,
        ..Default::default()
    }
}
pub(crate) fn fixture() -> (tempfile::TempDir, ReplicaWriter) {
    let directory = tempfile::tempdir().unwrap();
    let mut writer = ReplicaWriter::open(directory.path(), "test").unwrap();
    let mut builder = writer.begin_snapshot().unwrap();
    builder
        .append(&[
            entity(w::entity::Entity::Venue(w::Venue {
                id: "venue".into(),
                name: "Venue".into(),
            })),
            entity(w::entity::Entity::Currency(w::Currency {
                id: "GRAM".into(),
                name: "Gram".into(),
                decimals: 9,
            })),
            entity(w::entity::Entity::ItemType(w::ItemType {
                id: "gift".into(),
                name: "Gift".into(),
                aliases: vec![w::ExternalId {
                    namespace: "tg".into(),
                    id: "99".into(),
                }],
                quantity_step: "1".into(),
                ..Default::default()
            })),
            order("3"),
        ])
        .unwrap();
    writer
        .install_snapshot(builder.finish("c0").unwrap())
        .unwrap();
    (directory, writer)
}

#[tokio::test]
async fn public_actor_contract_serves_typed_queries_results_and_subscription_errors() {
    use crate::error::DataErrorCode;
    use market_gifts as m;
    let (_directory, mut writer) = fixture();
    writer
        .apply(w::Batch {
            cursor: "c1".into(),
            changes: vec![entity(w::entity::Entity::Feed(w::Feed {
                id: "source-feed".into(),
                books: vec![w::BookKey {
                    venue_id: "venue".into(),
                    item_type_id: "gift".into(),
                    currency_id: "GRAM".into(),
                }],
                modes: vec![w::OrderMode::Ask as i32],
                ..Default::default()
            }))],
        })
        .unwrap();
    let service = TestData::start(writer.reader(), 2, 1).await.unwrap();
    let query = m::GiftOrdersQuery::new(m::GiftFilters::new("gift".into(), "GRAM".into()));
    let page = service.handle.orders(query.clone()).await.unwrap();
    assert_eq!(page.cursor, "c1");
    assert_eq!(page.data[0].price.as_str(), "3");
    assert_eq!(page.data[0].kind, m::GiftOrderKind::Ask);
    let catalog = service.handle.catalog_snapshot().await.unwrap().data;
    assert!(service.handle.model_documents(1).await.unwrap().is_empty());
    for limit in [0, 1001] {
        assert_eq!(
            service
                .handle
                .model_documents(limit)
                .await
                .unwrap_err()
                .code(),
            DataErrorCode::InvalidQuery
        );
    }
    assert_eq!(catalog.books[0], page.data[0].book);
    assert_eq!(catalog.collections[0].id.as_str(), "gift");
    assert_eq!(
        service
            .handle
            .orders(m::GiftOrdersQuery::new(m::GiftFilters::new(
                "missing".into(),
                "GRAM".into()
            )))
            .await
            .err()
            .unwrap()
            .code(),
        DataErrorCode::InvalidQuery
    );
    let (lease, mut updates) = service.handle.test_window(query).await.unwrap();
    assert_eq!(
        next_window(&mut updates, &writer.reader())
            .await
            .data
            .iter()
            .map(|e| e.order.clone())
            .collect::<Vec<_>>(),
        page.data
    );
    drop(lease);
    terminal(&mut updates).await;
    let DataUpdate::Error(error) = updates.borrow().clone() else {
        panic!("missing terminal error")
    };
    assert_eq!(error.code(), DataErrorCode::Unavailable);
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}
async fn next_window(
    updates: &mut watch::Receiver<DataUpdate>,
    reader: &market_replica::ReplicaReader,
) -> Arc<market_gifts::OrderResult> {
    let expected = reader.read_revision(&raw_query()).unwrap().token();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let update = updates.borrow_and_update().clone();
            if let DataUpdate::Orders {
                result: page,
                revision,
                ..
            } = update
                && revision == expected
            {
                return page;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn independent_sync_writer_publishes_data_to_shared_query_and_subscription_actors() {
    let (_directory, mut writer) = fixture();
    let service = TestData::start(writer.reader(), 2, 2).await.unwrap();
    let api = service.handle.clone();
    let bot = service.handle.clone();
    assert_eq!(api.orders(gift_query()).await.unwrap().cursor, "c0");
    assert!(bot.status().await.unwrap().ready);
    let (_api_feed, mut api_updates) = api.test_window(gift_query()).await.unwrap();
    let (_bot_feed, mut bot_updates) = bot.test_window(gift_query()).await.unwrap();
    assert!(
        !api_updates.same_channel(&bot_updates),
        "both transports use independent range-watch lifetimes"
    );
    next_window(&mut api_updates, &writer.reader()).await;
    let mut another = gift_query();
    another.limit = 1;
    assert_eq!(
        api.test_window(another).await.err().unwrap().code(),
        crate::error::DataErrorCode::CapacityExceeded
    );
    writer
        .apply(w::Batch {
            cursor: "c1".into(),
            changes: vec![order("1")],
        })
        .unwrap();
    let page = next_window(&mut bot_updates, &writer.reader()).await;
    assert_eq!(page.data[0].order.price.as_str(), "1");
    assert_eq!(api.orders(gift_query()).await.unwrap().cursor, "c1");
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
    assert!(api.orders(gift_query()).await.is_err());
    assert!(bot.status().await.is_err());
    assert!(matches!(*api_updates.borrow(), DataUpdate::Error(_)));
}
#[derive(Message)]
#[rtype(result = "Result<ReadResult,Status>")]
struct SlowRead {
    permit: market_replica::ReadPermit,
    started: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}
impl Handler<SlowRead> for OrderbooksActor {
    type Result = ResponseFuture<Result<ReadResult, Status>>;
    fn handle(&mut self, message: SlowRead, _: &mut Self::Context) -> Self::Result {
        self.environment.blocking(message.permit, move |reader| {
            let _ = message.started.send(());
            message.release.recv().unwrap();
            reader.read(raw_query())
        })
    }
}
#[tokio::test]
async fn cancelled_api_request_retains_read_capacity_while_actor_still_answers_bot_status() {
    let (_directory, writer) = fixture();
    let service = TestData::start(writer.reader(), 1, 1).await.unwrap();
    let permit = service.handle.orderbooks.budget.try_query().unwrap();
    let address = service.handle.orderbooks.address.clone();
    let (started, running) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let request = tokio::spawn(async move {
        address
            .send(SlowRead {
                permit,
                started,
                release: blocked,
            })
            .await
    });
    running.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), service.handle.status())
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    request.abort();
    let _ = request.await;
    assert_eq!(
        service
            .handle
            .orders(gift_query())
            .await
            .err()
            .unwrap()
            .code(),
        crate::error::DataErrorCode::CapacityExceeded
    );
    assert_eq!(
        service
            .handle
            .catalog_snapshot()
            .await
            .err()
            .unwrap()
            .code(),
        crate::error::DataErrorCode::CapacityExceeded
    );
    let stop = CancellationToken::new();
    stop.cancel();
    let mut shutdown = tokio::spawn(service.task.run(stop));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut shutdown)
            .await
            .is_err()
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), shutdown)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
#[derive(Message)]
#[rtype(result = "()")]
struct Panic;
impl Handler<Panic> for OrderbooksActor {
    type Result = ();
    fn handle(&mut self, _: Panic, _: &mut Self::Context) {
        panic!("simulated data actor failure")
    }
}
impl Handler<Panic> for CatalogActor {
    type Result = ();
    fn handle(&mut self, _: Panic, _: &mut Self::Context) {
        panic!("simulated data actor failure")
    }
}
impl Handler<Panic> for MarketSubscriptionsActor {
    type Result = ();
    fn handle(&mut self, _: Panic, _: &mut Self::Context) {
        panic!("simulated data actor failure")
    }
}
#[tokio::test]
async fn every_actor_failure_is_supervised_without_owning_or_stopping_storage_itself() {
    for role in 0..3 {
        let (_directory, mut writer) = fixture();
        let service = TestData::start(writer.reader(), 1, 1).await.unwrap();
        let monitor = tokio::spawn(service.task.run(CancellationToken::new()));
        let closed = match role {
            0 => service.handle.orderbooks.address.send(Panic).await,
            1 => service.handle.catalog.address.send(Panic).await,
            _ => service.handle.subscriptions.send(Panic).await,
        };
        assert!(closed.is_err());
        assert!(
            tokio::time::timeout(Duration::from_secs(3), monitor)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(service.handle.status().await.is_err());
        assert!(service.handle.catalog_snapshot().await.is_err());
        writer
            .apply(w::Batch {
                cursor: "c2".into(),
                changes: vec![order("2")],
            })
            .unwrap();
        assert_eq!(writer.reader().read(raw_query()).unwrap().page.cursor, "c2");
    }
}

#[tokio::test]
async fn subscriptions_waiting_for_capacity_are_terminated_during_shutdown() {
    let (_directory, writer) = fixture();
    let service = TestData::start(writer.reader(), 1, 1).await.unwrap();
    let permit = service.handle.orderbooks.budget.try_query().unwrap();
    let (_feed, updates) = service.handle.test_window(gift_query()).await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(3), service.task.run(stop))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(*updates.borrow(), DataUpdate::Error(_)));
    drop(permit);
}

#[tokio::test]
async fn catalog_and_orderbooks_serve_distinct_typed_projections() {
    let (_directory, mut writer) = fixture();
    let service = TestData::start(writer.reader(), 2, 2).await.unwrap();
    let handle = &service.handle;
    // Private actor messages still reject another responsibility's generic selection.
    let invalid = handle
        .orderbooks
        .address
        .send(crate::orderbooks::messages::ReadOrders {
            request: w::ReadRequest {
                selection: Some(w::Selection {
                    kinds: vec![w::EntityKind::Venue as i32],
                    ..Default::default()
                }),
                ..Default::default()
            },
            permit: handle.orderbooks.budget.try_query().unwrap(),
        })
        .await
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(invalid.code(), tonic::Code::InvalidArgument);
    let before = handle.catalog_snapshot().await.unwrap();
    assert_eq!(before.data.collections[0].name, "Gift");
    assert_eq!(before.data.books.len(), 1);
    writer
        .apply(w::Batch {
            cursor: "c1".into(),
            changes: vec![order("1")],
        })
        .unwrap();
    let after = handle.catalog_snapshot().await.unwrap();
    assert_eq!(
        before.revision, after.revision,
        "Order prices do not invalidate catalog data"
    );
    assert_eq!(before.data, after.data);
    let orders = handle.orders(gift_query()).await.unwrap();
    assert_eq!(orders.cursor, "c1");
    assert_eq!(orders.data[0].price.as_str(), "1");
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}
async fn terminal(updates: &mut watch::Receiver<DataUpdate>) {
    tokio::time::timeout(Duration::from_secs(3), async {
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
#[tokio::test]
async fn market_watch_leases_cancel_independently_and_release_capacity() {
    let (_directory, mut writer) = fixture();
    let service = TestData::start(writer.reader(), 2, 2).await.unwrap();
    let (api, mut api_updates) = service.handle.test_window(gift_query()).await.unwrap();
    let (bot, mut bot_updates) = service.handle.test_window(gift_query()).await.unwrap();
    assert!(!api_updates.same_channel(&bot_updates));
    next_window(&mut api_updates, &writer.reader()).await;
    next_window(&mut bot_updates, &writer.reader()).await;
    assert_eq!(
        service
            .handle
            .test_window(gift_query())
            .await
            .err()
            .unwrap()
            .code(),
        crate::error::DataErrorCode::CapacityExceeded
    );
    drop(api);
    terminal(&mut api_updates).await;
    writer
        .apply(w::Batch {
            cursor: "changed".into(),
            changes: vec![order("1")],
        })
        .unwrap();
    let next = next_window(&mut bot_updates, &writer.reader()).await;
    assert_eq!(next.data[0].order.price.as_str(), "1");
    let (replacement, _) = service.handle.test_window(gift_query()).await.unwrap();
    drop((bot, replacement));
    terminal(&mut bot_updates).await;
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn gift_snapshot_pages_survive_requests_and_catalog_replacement() {
    use market_gifts::{GiftFilters, GiftHistoryQuery, GiftOrdersQuery, ReadConsistency};
    let (_directory, mut writer) = fixture();
    let row = |id: &str, mode: w::OrderMode, price: &str| {
        let mut change = order(price);
        if let Some(w::change::Change::Upsert(w::Entity {
            entity: Some(w::entity::Entity::Order(order)),
        })) = &mut change.change
        {
            order.id = id.into();
            order.mode = mode as i32;
            order.executed_at_ms = (mode == w::OrderMode::Executed).then_some(1000);
        }
        change
    };
    writer
        .apply(w::Batch {
            cursor: "pages".into(),
            changes: vec![
                row("a", w::OrderMode::Ask, "1"),
                row("b", w::OrderMode::Ask, "2"),
                row("execution-a", w::OrderMode::Executed, "1"),
                row("execution-b", w::OrderMode::Executed, "2"),
            ],
        })
        .unwrap();
    let service = TestData::start(writer.reader(), 2, 2).await.unwrap();
    let mut query = GiftOrdersQuery::new(GiftFilters::new("gift".into(), "GRAM".into()));
    query.limit = 1;
    let first = service.handle.orders(query.clone()).await.unwrap();
    let mut history = GiftHistoryQuery::new(query.filters.clone());
    history.limit = 1;
    let first_history = service.handle.executions(history.clone()).await.unwrap();
    assert!(!first.next_page_token.is_empty());
    assert!(!first_history.next_page_token.is_empty());
    // The current catalog no longer describes a gift, but the retained pages still do.
    writer
        .apply(w::Batch {
            cursor: "replacement".into(),
            changes: vec![
                row("b", w::OrderMode::Ask, "99"),
                entity(w::entity::Entity::ItemType(w::ItemType {
                    id: "gift".into(),
                    name: "Generic asset".into(),
                    quantity_step: "1".into(),
                    ..Default::default()
                })),
            ],
        })
        .unwrap();
    query.page_token = first.next_page_token;
    let second = service.handle.orders(query.clone()).await.unwrap();
    assert_eq!(second.data[0].id.as_str(), "b");
    assert_eq!(second.data[0].price.as_str(), "2");
    assert_eq!(second.cursor, "pages");
    history.page_token = first_history.next_page_token;
    let second_history = service.handle.executions(history).await.unwrap();
    assert_eq!(second_history.data[0].id.as_str(), "execution-b");
    assert_eq!(second_history.cursor, "pages");
    query.consistency = ReadConsistency::Live;
    assert_eq!(
        service.handle.orders(query).await.unwrap_err().code(),
        crate::error::DataErrorCode::InvalidQuery
    );
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn saturated_snapshot_capacity_leaves_normal_reads_available() {
    let (_dir, writer) = fixture();
    let service = TestData::start(writer.reader(), 4, 1).await.unwrap();
    let (busy, busy_task) = hold_read(service.reads.clone()).await;
    let reader = service.reads.clone();
    let mut work =
        tokio::spawn(async move { reader.prepare(|snapshot, _| snapshot.cursor()).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut work)
            .await
            .is_err()
    );
    assert!(service.handle.orders(gift_query()).await.is_ok());
    assert!(service.handle.catalog_snapshot().await.is_ok());
    drop(busy);
    busy_task.await.unwrap();
    work.await.unwrap().unwrap().unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn freshness_does_not_wait_for_query_capacity_and_snapshot_admission_is_isolated() {
    let (_directory, writer) = fixture();
    let service = TestData::start(writer.reader(), 2, 1).await.unwrap();
    let handle = service.handle.clone();
    let (_lease, mut updates) = handle.test_window(gift_query()).await.unwrap();
    let original = next_window(&mut updates, &writer.reader()).await;
    let query_a = handle.orderbooks.budget.try_query().unwrap();
    let query_b = handle.orderbooks.budget.try_query().unwrap();
    writer.mark(market_replica::SyncState::Stale);
    let (same, sync) = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let DataUpdate::Orders {
                result: page, sync, ..
            } = updates.borrow_and_update().clone()
                && sync.state == market_gifts::SyncState::Stale
            {
                break (page, sync);
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(Arc::ptr_eq(&original, &same));
    assert!(sync.ready);
    drop((query_a, query_b));
    let (heavy, heavy_task) = hold_read(service.reads.clone()).await;
    let reader = service.reads.clone();
    let mut work =
        tokio::spawn(async move { reader.prepare(|snapshot, _| snapshot.cursor()).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut work)
            .await
            .is_err()
    );
    assert_eq!(handle.orders(gift_query()).await.unwrap().cursor, "c0");
    drop(heavy);
    heavy_task.await.unwrap();
    work.await.unwrap().unwrap().unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn exhausted_orders_fill_pages_across_sorts_and_keep_snapshot_and_subscription_boundaries() {
    use market_gifts::{GiftFilters, GiftOrdersQuery, GiftSide, GiftSort, ReadConsistency};
    let (_directory, mut writer) = fixture();
    let mut changes = vec![entity(w::entity::Entity::Attribute(w::Attribute {
        id: "tg.number".into(),
        name: "Number".into(),
        value_type: w::AttributeType::Integer as i32,
        ..Default::default()
    }))];
    let row = |side: GiftSide, i: usize, exhausted: bool| {
        let mut value = w::Order {
            id: format!(
                "{}.{i:02}",
                if side == GiftSide::Ask { "ask" } else { "bid" }
            ),
            book: Some(w::BookKey {
                venue_id: "venue".into(),
                item_type_id: "gift".into(),
                currency_id: "GRAM".into(),
            }),
            mode: if side == GiftSide::Ask {
                w::OrderMode::Ask
            } else {
                w::OrderMode::Bid
            } as i32,
            price: (i / 3 + 1).to_string(),
            quantity: if exhausted { "0.00" } else { "1" }.into(),
            observed_at_ms: (i / 3 + 1) as i64,
            ..Default::default()
        };
        value.attributes.insert(
            "tg.number".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::Integer((i / 3 + 1) as i64)),
                })),
            },
        );
        entity(w::entity::Entity::Order(value))
    };
    // Retained generic state still contains the original exhausted order too.
    let mut original = order("3");
    if let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(value)),
    })) = &mut original.change
    {
        value.quantity = "0".into();
    }
    changes.push(original);
    for side in [GiftSide::Ask, GiftSide::Bid] {
        for i in 0..7 {
            changes.push(row(side, i, i % 2 == 0));
        }
    }
    writer
        .apply(w::Batch {
            changes,
            cursor: "exhausted".into(),
        })
        .unwrap();
    assert!(writer.reader().read(raw_query()).unwrap().page.entities.iter().any(|entity| matches!(&entity.entity,Some(w::entity::Entity::Order(o)) if o.quantity == "0" || o.quantity == "0.00")));
    let service = TestData::start(writer.reader(), 2, 2).await.unwrap();
    for side in [GiftSide::Ask, GiftSide::Bid] {
        for consistency in [ReadConsistency::Snapshot, ReadConsistency::Live] {
            for sort in [
                GiftSort::Identity,
                GiftSort::PriceAsc,
                GiftSort::PriceDesc,
                GiftSort::TimeAsc,
                GiftSort::TimeDesc,
                GiftSort::NumberAsc,
                GiftSort::NumberDesc,
            ] {
                let mut query =
                    GiftOrdersQuery::new(GiftFilters::new("gift".into(), "GRAM".into()));
                query.side = side;
                query.consistency = consistency;
                query.sort = sort;
                query.limit = 2;
                let first = service.handle.orders(query.clone()).await.unwrap();
                assert_eq!(first.data.len(), 2, "{side:?} {consistency:?} {sort:?}");
                assert!(!first.next_page_token.is_empty());
                query.page_token = first.next_page_token;
                let second = service.handle.orders(query).await.unwrap();
                assert_eq!(second.data.len(), 1);
                assert!(second.next_page_token.is_empty());
                let ids: Vec<_> = first
                    .data
                    .iter()
                    .chain(&second.data)
                    .map(|o| o.id.to_string())
                    .collect();
                let mut expected: Vec<_> = [1, 3, 5]
                    .into_iter()
                    .map(|i| {
                        format!(
                            "{}.{i:02}",
                            if side == GiftSide::Ask { "ask" } else { "bid" }
                        )
                    })
                    .collect();
                if matches!(
                    sort,
                    GiftSort::PriceDesc | GiftSort::TimeDesc | GiftSort::NumberDesc
                ) {
                    expected.reverse();
                }
                assert_eq!(ids, expected);
            }
        }
    }
    let mut query = GiftOrdersQuery::new(GiftFilters::new("gift".into(), "GRAM".into()));
    query.limit = 2;
    let retained = service.handle.orders(query.clone()).await.unwrap();
    let (lease, mut updates) = service.handle.test_window(query.clone()).await.unwrap();
    let head = next_window(&mut updates, &writer.reader()).await;
    assert_eq!(head.data.len(), 2);
    writer
        .apply(w::Batch {
            changes: (0..7).map(|i| row(GiftSide::Ask, i, true)).collect(),
            cursor: "empty".into(),
        })
        .unwrap();
    let empty = next_window(&mut updates, &writer.reader()).await;
    assert!(empty.data.is_empty());
    assert_eq!(empty.range, head.range);
    query.page_token = retained.next_page_token;
    let tail = service.handle.orders(query.clone()).await.unwrap();
    assert_eq!(tail.data[0].id.as_str(), "ask.05");
    assert_eq!(tail.cursor, "exhausted");
    query.page_token.clear();
    query.consistency = ReadConsistency::Live;
    assert!(service.handle.orders(query).await.unwrap().data.is_empty());
    drop(lease);
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

async fn hold_read(
    reads: market_replica::ReadExecutor,
) -> (std::sync::mpsc::Sender<()>, tokio::task::JoinHandle<()>) {
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    let task = tokio::spawn(async move {
        reads
            .prepare(move |_, _| {
                let _ = entered.send(());
                let _ = proceed.recv();
            })
            .await
            .unwrap();
    });
    started.await.unwrap();
    (release, task)
}

struct TestData {
    handle: MarketDataHandle,
    task: TestTask,
    reads: market_replica::ReadExecutor,
}
struct TestTask {
    data: MarketDataTask,
    reads: market_replica::ReadRuntime,
}
impl TestData {
    async fn start(
        reader: market_replica::ReplicaReader,
        queries: usize,
        subscriptions: usize,
    ) -> Result<Self, Status> {
        let reads = market_replica::ReadRuntime::new(reader, queries, CancellationToken::new())?;
        let data = MarketDataService::start(reads.handle(), subscriptions).await?;
        Ok(Self {
            handle: data.handle,
            reads: reads.handle(),
            task: TestTask {
                data: data.task,
                reads,
            },
        })
    }
}
impl TestTask {
    async fn run(self, stop: CancellationToken) -> Result<(), Status> {
        let result = self.data.run(stop).await;
        self.reads.shutdown().await;
        result
    }
}

#[tokio::test]
async fn range_watches_keep_open_edges_freshness_and_independent_leases() {
    let (_dir, mut writer) = fixture();
    let service = TestData::start(writer.reader(), 4, 2).await.unwrap();
    let query = Seed {
        filters: market_gifts::GiftFilters::new("gift".into(), "GRAM".into()),
        side: Default::default(),
        sort: Default::default(),
        anchor: None,
        before: 1,
        after: 1,
    };
    let (a, mut updates) = service.handle.test_seed_watch(query.clone()).await.unwrap();
    let (b, other) = service.handle.test_seed_watch(query.clone()).await.unwrap();
    assert!(!updates.same_channel(&other));
    async fn next(
        updates: &mut watch::Receiver<DataUpdate>,
        price: &str,
    ) -> Arc<market_gifts::OrderResult> {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let DataUpdate::Orders { result: window, .. } =
                    updates.borrow_and_update().clone()
                    && window
                        .data
                        .first()
                        .is_some_and(|e| e.order.price.as_str() == price)
                {
                    break window;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap()
    }
    let first = next(&mut updates, "3").await;
    drop(b);
    writer
        .apply(w::Batch {
            cursor: "window2".into(),
            changes: vec![order("2")],
        })
        .unwrap();
    let changed = next(&mut updates, "2").await;
    assert!(!Arc::ptr_eq(&first, &changed));
    let permits = (0..4)
        .map(|_| service.handle.orderbooks.budget.try_query().unwrap())
        .collect::<Vec<_>>();
    writer.mark(market_replica::SyncState::Stale);
    let same = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let DataUpdate::Orders {
                result: window,
                sync,
                ..
            } = updates.borrow_and_update().clone()
                && sync.state == market_gifts::SyncState::Stale
            {
                break window;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(Arc::ptr_eq(&changed, &same));
    drop(permits);
    let changes = (0..4)
        .map(|i| {
            let w::change::Change::Upsert(mut value) =
                order(&format!("0.{}", i + 1)).change.unwrap()
            else {
                unreachable!()
            };
            let Some(w::entity::Entity::Order(row)) = &mut value.entity else {
                unreachable!()
            };
            row.id = format!("new{i}");
            w::Change {
                change: Some(w::change::Change::Upsert(value)),
            }
        })
        .collect();
    writer
        .apply(w::Batch {
            changes,
            cursor: "head-moved".into(),
        })
        .unwrap();
    let head = next(&mut updates, "0.1").await;
    assert_eq!(
        head.data.len(),
        5,
        "An open range grows to include new rows rather than evicting its tail"
    );

    drop(a);
    drop(other);
    drop(updates);
    let mut different = query;
    different.before = 2;
    let (lease, _) = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(result) = service.handle.test_seed_watch(different.clone()).await {
                break result;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(lease);
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[derive(Clone)]
struct Seed {
    filters: market_gifts::GiftFilters,
    side: market_gifts::GiftSide,
    sort: market_gifts::GiftSort,
    anchor: Option<String>,
    before: u32,
    after: u32,
}
impl Seed {
    fn query(self) -> market_gifts::GiftOrderQuery {
        market_gifts::GiftOrderQuery {
            filters: self.filters,
            source: if self.side == market_gifts::GiftSide::Bid {
                market_gifts::OrderSource::Bids
            } else {
                market_gifts::OrderSource::Asks
            },
            sort: self.sort,
            position: self.anchor,
            limit: self.before + self.after + 1,
            period: None,
            direction: if self.before == 0 {
                market_gifts::ReadDirection::Forward
            } else if self.after == 0 {
                market_gifts::ReadDirection::Backward
            } else {
                market_gifts::ReadDirection::Around
            },
        }
    }
}
impl MarketDataHandle {
    async fn test_seed_watch(
        &self,
        seed: Seed,
    ) -> crate::error::Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>)> {
        self.watch_orders(market_gifts::OrderWatchQuery {
            query: seed.query(),
            range: None,
        })
        .await
    }
    async fn test_window(
        &self,
        query: market_gifts::GiftOrdersQuery,
    ) -> crate::error::Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>)> {
        self.test_seed_watch(Seed {
            filters: query.filters,
            side: query.side,
            sort: query.sort,
            anchor: None,
            before: 0,
            after: query.limit - 1,
        })
        .await
    }
}

#[tokio::test]
async fn fixed_ranges_remove_exiting_rows_and_new_watches_seed_independently() {
    let (_dir, mut writer) = fixture();
    let changes = [("a", "10"), ("b", "11"), ("c", "12")]
        .into_iter()
        .map(|(id, price)| {
            let mut change = order(price);
            let Some(w::change::Change::Upsert(ref mut value)) = change.change else {
                unreachable!()
            };
            let Some(w::entity::Entity::Order(ref mut row)) = value.entity else {
                unreachable!()
            };
            row.id = id.into();
            change
        })
        .collect();
    writer
        .apply(w::Batch {
            changes,
            cursor: "ready".into(),
        })
        .unwrap();
    let service = TestData::start(writer.reader(), 4, 5).await.unwrap();
    let mut query = Seed {
        filters: market_gifts::GiftFilters::new("gift".into(), "GRAM".into()),
        side: Default::default(),
        sort: Default::default(),
        anchor: None,
        before: 0,
        after: 3,
    };
    let (head, mut head_updates) = service.handle.test_seed_watch(query.clone()).await.unwrap();
    let initial = next_window(&mut head_updates, &writer.reader()).await;
    query.anchor = Some(
        initial
            .data
            .iter()
            .find(|e| e.order.id.as_str() == "a")
            .unwrap()
            .position
            .clone(),
    );
    query.after = 0;
    let (first, mut first_updates) = service.handle.test_seed_watch(query.clone()).await.unwrap();
    next_window(&mut first_updates, &writer.reader()).await;
    writer
        .apply(w::Batch {
            cursor: "gone".into(),
            changes: vec![w::Change {
                change: Some(w::change::Change::Remove(w::Removal {
                    key: Some(w::Key {
                        kind: 7,
                        venue_id: "venue".into(),
                        id: "a".into(),
                    }),
                    ..Default::default()
                })),
            }],
        })
        .unwrap();
    let empty = next_window(&mut first_updates, &writer.reader()).await;
    assert!(
        empty.data.is_empty(),
        "a fixed single-row range does not refill after deletion"
    );
    let mut moved = order("100");
    let Some(w::change::Change::Upsert(ref mut value)) = moved.change else {
        unreachable!()
    };
    let Some(w::entity::Entity::Order(ref mut row)) = value.entity else {
        unreachable!()
    };
    row.id = "b".into();
    writer
        .apply(w::Batch {
            cursor: "fallback-moved".into(),
            changes: vec![moved],
        })
        .unwrap();
    assert!(
        next_window(&mut first_updates, &writer.reader())
            .await
            .data
            .is_empty()
    );
    let (late, mut late_updates) = service.handle.test_seed_watch(query).await.unwrap();
    assert!(!late_updates.same_channel(&first_updates));
    assert_eq!(
        next_window(&mut late_updates, &writer.reader()).await.data[0]
            .order
            .id
            .as_str(),
        "c"
    );
    drop((head, first, late));
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn frozen_windows_reject_overload_while_live_producers_wait_and_cleanup_stays_available() {
    let (_directory, writer) = fixture();
    let service = TestData::start(writer.reader(), 1, 1).await.unwrap();
    let query = Seed {
        filters: gift_query().filters,
        side: Default::default(),
        sort: Default::default(),
        anchor: None,
        before: 0,
        after: 49,
    };
    let frozen = market_gifts::OrderReadQuery {
        consistency: market_gifts::ReadView::Snapshot,
        query: query.clone().query(),
        snapshot: None,
    };
    let first = service.handle.read_orders(frozen.clone()).await.unwrap();
    let permit = service.handle.orderbooks.budget.try_query().unwrap();
    let error = tokio::time::timeout(
        Duration::from_millis(100),
        service.handle.read_orders(frozen),
    )
    .await
    .expect("Foreground reads must reject overload without waiting")
    .err()
    .unwrap();
    assert_eq!(error.code(), crate::error::DataErrorCode::CapacityExceeded);
    // Cleanup and cheap status do not require read slots.
    service
        .handle
        .release_read_snapshot(first.snapshot.unwrap())
        .await
        .unwrap();
    assert!(service.handle.status().await.unwrap().ready);
    let (lease, mut updates) = service.handle.test_seed_watch(query).await.unwrap();
    assert!(matches!(*updates.borrow(), DataUpdate::Connecting));
    drop(permit);
    assert_eq!(
        next_window(&mut updates, &writer.reader()).await.data.len(),
        1
    );
    drop(lease);
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn identical_dataset_replacement_changes_watch_epoch_without_losing_coverage() {
    let (_dir, mut writer) = fixture();
    let service = TestData::start(writer.reader(), 2, 2).await.unwrap();
    let (lease, mut updates) = service.handle.test_window(gift_query()).await.unwrap();
    let first = next_window(&mut updates, &writer.reader()).await;
    let DataUpdate::Orders {
        epoch: initial_epoch,
        ..
    } = updates.borrow_and_update().clone()
    else {
        panic!("missing range")
    };
    let page = writer
        .reader()
        .read(w::ReadRequest {
            mode: w::ReadMode::Live as i32,
            limit: 500,
            ..Default::default()
        })
        .unwrap()
        .page;
    let mut builder = writer.begin_snapshot().unwrap();
    builder
        .append(
            &page
                .entities
                .into_iter()
                .map(|entity| w::Change {
                    change: Some(w::change::Change::Upsert(entity)),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    writer
        .install_snapshot(builder.finish("replaced").unwrap())
        .unwrap();
    let next = next_window(&mut updates, &writer.reader()).await;
    let DataUpdate::Orders { epoch, .. } = updates.borrow_and_update().clone() else {
        panic!("missing range")
    };
    assert_ne!(epoch, initial_epoch);
    assert!(
        Arc::ptr_eq(&first, &next),
        "unchanged rows can share data while the transport emits an explicit reset"
    );
    assert_eq!(next.range, first.range);
    drop(lease);
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[test]
fn query_projection_rejects_duplicate_identities_instead_of_silently_collapsing_changes() {
    let (_dir, writer) = fixture();
    let mut request = raw_query();
    request.mode = w::ReadMode::Live as i32;
    let mut read = writer
        .reader()
        .read_window_checked(
            request,
            market_replica::WindowRequest {
                fill_edges: true,
                anchor: None,
                before: 0,
                after: 1,
                snapshot: None,
            },
            crate::query::validate_order_collection,
            crate::protocol::include_order,
        )
        .unwrap();
    read.entries.push(read.entries[0].clone());
    assert_eq!(
        crate::protocol::order_result(read, None)
            .unwrap_err()
            .code(),
        crate::error::DataErrorCode::Unavailable
    );
}
