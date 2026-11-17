use crate::*;
use crate::{
    catalog::CatalogActor, orderbooks::OrderbooksActor, subscriptions::UserSubscriptionsActor,
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
fn entity(value: w::entity::Entity) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Upsert(w::Entity {
            entity: Some(value),
        })),
    }
}
fn order(price: &str) -> w::Change {
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
fn query() -> w::ReadRequest {
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
fn fixture() -> (tempfile::TempDir, ReplicaWriter) {
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
    let service = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let query = m::GiftOrdersQuery::new(m::GiftFilters::new("gift".into(), "GRAM".into()));
    let page = service.handle.orders(query.clone()).await.unwrap();
    assert_eq!(page.cursor, "c1");
    assert_eq!(page.data[0].price.as_str(), "3");
    assert_eq!(page.data[0].mode, m::GiftOrderKind::Ask);
    let catalog = service.handle.catalog().await.unwrap();
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
    let (lease, mut updates) = service.handle.subscribe(query).await.unwrap();
    assert_eq!(next_page(&mut updates, "c1").await.data, page.data);
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
async fn next_page(
    updates: &mut watch::Receiver<DataUpdate>,
    cursor: &str,
) -> Arc<market_gifts::Page<market_gifts::GiftOrder>> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let update = updates.borrow_and_update().clone();
            if let DataUpdate::View { page, .. } = update
                && page.cursor == cursor
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
    let service = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let api = service.handle.clone();
    let bot = service.handle.clone();
    assert_eq!(api.orders_raw(query()).await.unwrap().page.cursor, "c0");
    assert!(bot.status().await.unwrap().ready);
    let (_api_feed, mut api_updates) = api.subscribe_raw(query()).await.unwrap();
    let (_bot_feed, mut bot_updates) = bot.subscribe_raw(query()).await.unwrap();
    assert!(
        api_updates.same_channel(&bot_updates),
        "both transports share the actor's subscription registry"
    );
    next_page(&mut api_updates, "c0").await;
    let mut another = query();
    another.limit = 1;
    assert_eq!(
        api.subscribe_raw(another).await.err().unwrap().code(),
        tonic::Code::ResourceExhausted
    );
    writer
        .apply(w::Batch {
            cursor: "c1".into(),
            changes: vec![order("1")],
        })
        .unwrap();
    let page = next_page(&mut bot_updates, "c1").await;
    assert_eq!(page.data[0].price.as_str(), "1");
    assert_eq!(api.orders_raw(query()).await.unwrap().page.cursor, "c1");
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
    assert!(api.orders_raw(query()).await.is_err());
    assert!(bot.status().await.is_err());
    assert!(matches!(*api_updates.borrow(), DataUpdate::Error(_)));
}
#[derive(Message)]
#[rtype(result = "Result<ReadResult,Status>")]
struct SlowRead {
    permit: tokio::sync::OwnedSemaphorePermit,
    started: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}
impl Handler<SlowRead> for OrderbooksActor {
    type Result = ResponseFuture<Result<ReadResult, Status>>;
    fn handle(&mut self, message: SlowRead, _: &mut Self::Context) -> Self::Result {
        self.environment
            .blocking(self.reader.clone(), message.permit, move |reader| {
                let _ = message.started.send(());
                message.release.recv().unwrap();
                reader.read(query())
            })
    }
}
#[tokio::test]
async fn cancelled_api_request_retains_read_capacity_while_actor_still_answers_bot_status() {
    let (_directory, writer) = fixture();
    let service = MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
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
            .orders_raw(query())
            .await
            .err()
            .unwrap()
            .code(),
        tonic::Code::ResourceExhausted
    );
    assert_eq!(
        service
            .handle
            .catalog_raw(catalog_query())
            .await
            .err()
            .unwrap()
            .code(),
        tonic::Code::ResourceExhausted
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
impl Handler<Panic> for UserSubscriptionsActor {
    type Result = ();
    fn handle(&mut self, _: Panic, _: &mut Self::Context) {
        panic!("simulated data actor failure")
    }
}
#[tokio::test]
async fn every_actor_failure_is_supervised_without_owning_or_stopping_storage_itself() {
    for role in 0..3 {
        let (_directory, mut writer) = fixture();
        let service = MarketDataService::start(writer.reader(), 1, 1)
            .await
            .unwrap();
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
        assert!(service.handle.catalog_raw(catalog_query()).await.is_err());
        writer
            .apply(w::Batch {
                cursor: "c2".into(),
                changes: vec![order("2")],
            })
            .unwrap();
        assert_eq!(writer.reader().read(query()).unwrap().page.cursor, "c2");
    }
}

#[tokio::test]
async fn subscriptions_waiting_for_capacity_are_terminated_during_shutdown() {
    let (_directory, writer) = fixture();
    let service = MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let permit = service.handle.orderbooks.budget.try_query().unwrap();
    let (_feed, updates) = service.handle.subscribe_raw(query()).await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(3), service.task.run(stop))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(*updates.borrow(), DataUpdate::Error(_)));
    drop(permit);
}

fn catalog_query() -> w::ReadRequest {
    w::ReadRequest {
        selection: Some(w::Selection {
            kinds: vec![
                w::EntityKind::Venue as i32,
                w::EntityKind::Currency as i32,
                w::EntityKind::ItemType as i32,
            ],
            ..Default::default()
        }),
        limit: 1,
        ..Default::default()
    }
}
#[tokio::test]
async fn catalog_and_orderbooks_enforce_their_projection_and_preserve_snapshot_boundaries() {
    use crate::{catalog::messages::ReadCatalog, orderbooks::messages::ReadOrders};
    let (_directory, mut writer) = fixture();
    let service = MarketDataService::start(writer.reader(), 2, 2)
        .await
        .unwrap();
    let handle = &service.handle;
    assert_eq!(
        handle
            .orders_raw(catalog_query())
            .await
            .err()
            .unwrap()
            .code(),
        tonic::Code::InvalidArgument
    );
    assert_eq!(
        handle.catalog_raw(query()).await.err().unwrap().code(),
        tonic::Code::InvalidArgument
    );
    assert_eq!(
        handle
            .subscribe_raw(catalog_query())
            .await
            .err()
            .unwrap()
            .code(),
        tonic::Code::InvalidArgument
    );
    let permit = handle.orderbooks.budget.try_query().unwrap();
    assert_eq!(
        handle
            .orderbooks
            .address
            .send(ReadOrders {
                request: catalog_query(),
                permit
            })
            .await
            .unwrap()
            .err()
            .unwrap()
            .code(),
        tonic::Code::InvalidArgument
    );
    let permit = handle.catalog.budget.try_query().unwrap();
    assert_eq!(
        handle
            .catalog
            .address
            .send(ReadCatalog {
                request: query(),
                permit
            })
            .await
            .unwrap()
            .err()
            .unwrap()
            .code(),
        tonic::Code::InvalidArgument
    );
    let first = handle.catalog_raw(catalog_query()).await.unwrap();
    assert_eq!(first.page.cursor, "c0");
    assert!(!first.page.next_page_token.is_empty());
    assert!(
        first
            .page
            .entities
            .iter()
            .all(|entity| !matches!(entity.entity, Some(w::entity::Entity::Order(_))))
    );
    writer
        .apply(w::Batch {
            cursor: "c1".into(),
            changes: vec![order("1")],
        })
        .unwrap();
    let mut next = catalog_query();
    next.page_token = first.page.next_page_token;
    let page = handle.catalog_raw(next).await.unwrap();
    assert_eq!(page.page.cursor, "c0");
    assert_eq!(handle.orders_raw(query()).await.unwrap().page.cursor, "c1");
    assert_eq!(
        handle
            .books_raw(w::BooksRequest {
                limit: 50,
                ..Default::default()
            })
            .await
            .unwrap()
            .books
            .len(),
        1
    );
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
async fn user_subscription_leases_cancel_independently_and_last_release_stops_the_feed() {
    let (_directory, mut writer) = fixture();
    let service = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let (api_lease, mut api_updates) = service.handle.subscribe_raw(query()).await.unwrap();
    let (bot_lease, mut bot_updates) = service.handle.subscribe_raw(query()).await.unwrap();
    assert!(
        !Arc::ptr_eq(&api_lease, &bot_lease),
        "callers have separate leases"
    );
    assert!(
        api_updates.same_channel(&bot_updates),
        "identical queries share computation"
    );
    next_page(&mut bot_updates, "c0").await;
    drop(api_lease);
    writer
        .apply(w::Batch {
            cursor: "c1".into(),
            changes: vec![order("1")],
        })
        .unwrap();
    next_page(&mut bot_updates, "c1").await;
    let mut another = query();
    another.limit = 1;
    assert_eq!(
        service
            .handle
            .subscribe_raw(another.clone())
            .await
            .err()
            .unwrap()
            .code(),
        tonic::Code::ResourceExhausted
    );
    drop(bot_lease);
    // Retained receivers cannot keep computation alive after both callers unsubscribe.
    terminal(&mut api_updates).await;
    terminal(&mut bot_updates).await;
    let (_lease, mut updates) = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match service.handle.subscribe_raw(another.clone()).await {
                Ok(subscription) => break subscription,
                Err(error) if error.code() == tonic::Code::ResourceExhausted => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(error) => panic!("unexpected subscribe failure: {error}"),
            }
        }
    })
    .await
    .unwrap();
    next_page(&mut updates, "c1").await;
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn model_valuation_uses_exact_current_asks_and_reports_unpriced_gifts_separately() {
    use market_gifts::valuation::{ModelLookup, ModelPosition, UnpricedReason, ValuationRequest};
    let (_directory, mut writer) = fixture();
    let option = "gift.model.10";
    let ask = |id: &str,
               venue: &str,
               currency: &str,
               mode: w::OrderMode,
               price: &str,
               quantity: &str,
               expires_at_ms| {
        entity(w::entity::Entity::Order(w::Order {
            id: id.into(),
            book: Some(w::BookKey {
                venue_id: venue.into(),
                item_type_id: "gift".into(),
                currency_id: currency.into(),
            }),
            mode: mode as i32,
            price: price.into(),
            quantity: quantity.into(),
            expires_at_ms,
            executed_at_ms: (mode == w::OrderMode::Executed).then_some(1),
            attributes: [(
                "tg.model".into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId(option.into())),
                    })),
                },
            )]
            .into(),
            ..Default::default()
        }))
    };
    writer
        .apply(w::Batch {
            cursor: "c1".into(),
            changes: vec![
                entity(w::entity::Entity::Attribute(w::Attribute {
                    id: "tg.model".into(),
                    name: "Model".into(),
                    value_type: w::AttributeType::Enum as i32,
                    ..Default::default()
                })),
                entity(w::entity::Entity::AttributeOption(w::AttributeOption {
                    id: option.into(),
                    name: "Rare Model".into(),
                    attribute_id: "tg.model".into(),
                    aliases: vec![w::ExternalId {
                        namespace: "tg".into(),
                        id: "10".into(),
                    }],
                    ..Default::default()
                })),
                entity(w::entity::Entity::AttributeOption(w::AttributeOption {
                    id: "gift.model.20".into(),
                    name: "Unlisted Model".into(),
                    attribute_id: "tg.model".into(),
                    aliases: vec![w::ExternalId {
                        namespace: "tg".into(),
                        id: "20".into(),
                    }],
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
                    attributes: [(
                        "tg.model".into(),
                        w::AttributeRule {
                            required: true,
                            allowed_values: Some(w::AllowedAttributeValues {
                                ids: vec![option.into(), "gift.model.20".into()],
                            }),
                        },
                    )]
                    .into(),
                })),
                entity(w::entity::Entity::Currency(w::Currency {
                    id: "XTR".into(),
                    name: "Stars".into(),
                    decimals: 0,
                })),
                entity(w::entity::Entity::Venue(w::Venue {
                    id: "another".into(),
                    name: "Another venue".into(),
                })),
                ask("order", "venue", "GRAM", w::OrderMode::Ask, "3", "1", None),
                ask(
                    "cheapest",
                    "another",
                    "GRAM",
                    w::OrderMode::Ask,
                    "2.000000000000000001",
                    "1",
                    None,
                ),
                ask(
                    "expired",
                    "venue",
                    "GRAM",
                    w::OrderMode::Ask,
                    "0.1",
                    "1",
                    Some(1),
                ),
                ask(
                    "empty",
                    "venue",
                    "GRAM",
                    w::OrderMode::Ask,
                    "0.001",
                    "0",
                    None,
                ),
                ask("bid", "venue", "GRAM", w::OrderMode::Bid, "0.01", "1", None),
                ask(
                    "executed",
                    "venue",
                    "GRAM",
                    w::OrderMode::Executed,
                    "0.02",
                    "1",
                    None,
                ),
                ask(
                    "stars",
                    "venue",
                    "XTR",
                    w::OrderMode::Ask,
                    "0.0001",
                    "1",
                    None,
                ),
            ],
        })
        .unwrap();
    let pinned = writer.reader().snapshot().unwrap();
    writer
        .apply(w::Batch {
            cursor: "c2".into(),
            changes: vec![entity(w::entity::Entity::Venue(w::Venue {
                id: "venue".into(),
                name: "Updated venue".into(),
            }))],
        })
        .unwrap();
    assert_eq!(pinned.read(query()).unwrap().page.cursor, "c1");
    assert_eq!(writer.reader().read(query()).unwrap().page.cursor, "c2");
    let service = MarketDataService::start(writer.reader(), 2, 2)
        .await
        .unwrap();
    let lookup = ModelLookup::Name {
        collection: "gift".into(),
        model: "rare model".into(),
    };
    let value = service
        .handle
        .value_models(ValuationRequest {
            positions: vec![
                ModelPosition {
                    model: lookup.clone(),
                    quantity: 2,
                },
                ModelPosition {
                    model: ModelLookup::Telegram {
                        gift_id: market_gifts::TelegramGiftId::new(99).unwrap(),
                        document_id: market_gifts::TelegramDocumentId::new(20).unwrap(),
                    },
                    quantity: 1,
                },
                ModelPosition {
                    model: ModelLookup::Name {
                        collection: "gift".into(),
                        model: "Unknown".into(),
                    },
                    quantity: 1,
                },
            ],
            currency_id: "GRAM".into(),
        })
        .await
        .unwrap();
    assert_eq!(value.total.as_str(), "4.000000000000000002");
    assert_eq!((value.priced, value.unpriced), (2, 2));
    assert_eq!(value.cursor, "c2");
    assert_eq!(
        value
            .rows
            .iter()
            .find(|row| row.floor.is_some())
            .unwrap()
            .venue_id
            .as_deref(),
        Some("another")
    );
    assert!(
        value
            .rows
            .iter()
            .any(|row| row.unpriced == Some(UnpricedReason::NoListing))
    );
    assert!(
        value
            .rows
            .iter()
            .any(|row| row.unpriced == Some(UnpricedReason::UnknownModel))
    );
    let stars = service
        .handle
        .value_models(ValuationRequest {
            positions: vec![ModelPosition {
                model: lookup.clone(),
                quantity: 2,
            }],
            currency_id: "XTR".into(),
        })
        .await
        .unwrap();
    assert_eq!(stars.total.as_str(), "0.0002");
    assert!(
        service
            .handle
            .value_models(ValuationRequest {
                positions: vec![ModelPosition {
                    model: lookup,
                    quantity: 0
                }],
                currency_id: "GRAM".into()
            })
            .await
            .is_err()
    );
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
    let service = MarketDataService::start(writer.reader(), 2, 2)
        .await
        .unwrap();
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
async fn saturated_valuation_capacity_leaves_normal_reads_available() {
    let (_dir, writer) = fixture();
    let service = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let busy = service
        .handle
        .orderbooks
        .valuations
        .clone()
        .acquire_owned()
        .await
        .unwrap();
    let result = service
        .handle
        .orderbooks
        .value_models(market_gifts::valuation::ValuationRequest {
            currency_id: "GRAM".into(),
            positions: vec![],
        })
        .await;
    assert_eq!(result.unwrap_err().code(), tonic::Code::ResourceExhausted);
    assert!(service.handle.orders_raw(query()).await.is_ok());
    assert!(service.handle.catalog().await.is_ok());
    drop(busy);
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn freshness_does_not_wait_for_query_capacity_and_valuation_admission_is_isolated() {
    let (_directory, writer) = fixture();
    let service = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let handle = service.handle.clone();
    let (_lease, mut updates) = handle.subscribe_raw(query()).await.unwrap();
    let original = next_page(&mut updates, "c0").await;
    let query_a = handle.orderbooks.budget.try_query().unwrap();
    let query_b = handle.orderbooks.budget.try_query().unwrap();
    writer.mark(market_replica::SyncState::Stale);
    let (same, sync) = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let DataUpdate::View { page, sync, .. } = updates.borrow_and_update().clone()
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
    let heavy = handle
        .orderbooks
        .valuations
        .clone()
        .try_acquire_owned()
        .unwrap();
    let valuation = handle
        .orderbooks
        .value_models(market_gifts::valuation::ValuationRequest {
            positions: vec![],
            currency_id: "GRAM".into(),
        })
        .await;
    assert_eq!(
        valuation.unwrap_err().code(),
        tonic::Code::ResourceExhausted
    );
    // An occupied heavy slot does not consume ordinary read capacity.
    assert_eq!(handle.orders_raw(query()).await.unwrap().page.cursor, "c0");
    drop(heavy);
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}
