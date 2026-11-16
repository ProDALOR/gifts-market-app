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
    use crate::{error::DataErrorCode, models as m};
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
    let query = m::OrdersQuery {
        scope: m::Scope {
            item_type_id: Some("gift".into()),
            currency_id: Some("GRAM".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let page = service.handle.orders(query.clone()).await.unwrap();
    assert_eq!(page.cursor, "c1");
    assert_eq!(page.data[0].price, "3");
    assert_eq!(page.data[0].mode, m::Mode::Ask);
    assert_eq!(
        service
            .handle
            .books(m::BooksQuery::default())
            .await
            .unwrap()
            .data[0],
        page.data[0].book
    );
    let catalog = service
        .handle
        .catalog(m::CatalogQuery {
            kind: m::CatalogKind::Feed,
            ids: vec!["source-feed".into()],
            limit: 50,
            page_token: String::new(),
        })
        .await
        .unwrap();
    assert!(
        matches!(&catalog.data[0], m::CatalogEntry::Feed { id, books, modes, .. }
        if id == "source-feed" && books[0] == page.data[0].book && modes == &[m::Mode::Ask])
    );
    assert_eq!(
        service
            .handle
            .orders(m::OrdersQuery::default())
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
) -> Arc<models::Page<models::Order>> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let update = updates.borrow_and_update().clone();
            if let DataUpdate::Page(page) = update
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
    assert_eq!(page.data[0].price, "1");
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
    use crate::valuation::{ModelLookup, ModelPosition, UnpricedReason, ValuationRequest};
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
    let lookup = ModelLookup {
        namespace: None,
        collection: "gift".into(),
        attribute_id: "tg.model".into(),
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
                    model: ModelLookup {
                        namespace: Some("tg".into()),
                        collection: "99".into(),
                        attribute_id: "tg.model".into(),
                        model: "20".into(),
                    },
                    quantity: 1,
                },
                ModelPosition {
                    model: ModelLookup {
                        model: "Unknown".into(),
                        ..lookup.clone()
                    },
                    quantity: 1,
                },
            ],
            currency_id: "GRAM".into(),
        })
        .await
        .unwrap();
    assert_eq!(value.total, "4.000000000000000002");
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
    assert_eq!(stars.total, "0.0002");
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
