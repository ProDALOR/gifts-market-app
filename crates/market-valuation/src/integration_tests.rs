use crate::ValuationSource;
#[cfg(feature = "portfolio")]
use crate::portfolio::calculation;
use crate::test_support::MarketDataService;
use crate::*;
use actix::prelude::*;
#[cfg(feature = "portfolio")]
use market_gifts::ModelLookup;
use market_hub_protocol::wire as w;
use market_replica::ReplicaWriter;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
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
#[cfg(feature = "portfolio")]
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

#[cfg(feature = "portfolio")]
#[tokio::test]
async fn model_valuation_uses_exact_current_asks_and_reports_unpriced_gifts_separately() {
    use crate::{ModelPosition, UnpricedReason, ValuationRequest};
    #[cfg(feature = "portfolio")]
    use market_gifts::ModelLookup;
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
    let value = calculate(
        &service.reader,
        ValuationRequest {
            strategy: crate::StrategyId::model_floor(),
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
        },
    )
    .await
    .unwrap();
    assert_eq!(value.total.as_str(), "4.000000000000000002");
    assert_eq!((value.priced, value.unpriced), (2, 2));
    assert_eq!(value.revision, "c2");
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
    let stars = calculate(
        &service.reader,
        ValuationRequest {
            strategy: crate::StrategyId::model_floor(),
            positions: vec![ModelPosition {
                model: lookup.clone(),
                quantity: 2,
            }],
            currency_id: "XTR".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(stars.total.as_str(), "0.0002");
    assert!(
        calculate(
            &service.reader,
            ValuationRequest {
                strategy: crate::StrategyId::model_floor(),
                positions: vec![ModelPosition {
                    model: lookup,
                    quantity: 0
                }],
                currency_id: "GRAM".into()
            }
        )
        .await
        .is_err()
    );
    let stop = CancellationToken::new();
    stop.cancel();
    service.task.run(stop).await.unwrap();
}

#[cfg(feature = "portfolio")]
#[tokio::test]
async fn strategy_registry_shares_read_budget_and_preserves_typed_failures() {
    use crate::ValuationErrorCode;
    use crate::{FloorActor, PortfolioValuationHandle};
    use crate::{ModelPosition, StrategyId, ValuationRequest};
    #[cfg(feature = "portfolio")]
    use market_gifts::ModelLookup;
    let (_directory, mut writer) = fixture();
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let model = crate::Service::start_on(
        &data.arbiter(),
        "model_floor",
        {
            let source = data.reader.clone();
            move |life| FloorActor::model(source, life)
        },
        Default::default(),
    )
    .await
    .unwrap();
    let collection = crate::Service::start_on(
        &data.arbiter(),
        "collection_floor",
        {
            let source = data.reader.clone();
            move |life| FloorActor::collection(source, life)
        },
        Default::default(),
    )
    .await
    .unwrap();
    assert!(PortfolioValuationHandle::new([] as [crate::FloorStrategyHandle; 0]).is_err());
    assert!(PortfolioValuationHandle::new([model.handle.clone(), model.handle.clone()]).is_err());
    let registry =
        PortfolioValuationHandle::new([model.handle.clone(), collection.handle.clone()]).unwrap();
    assert_eq!(registry.strategies().count(), 2);
    let request = ValuationRequest {
        strategy: StrategyId::model_floor(),
        currency_id: "GRAM".into(),
        positions: vec![ModelPosition {
            model: ModelLookup::named("Gift", "Missing").unwrap(),
            quantity: 2,
        }],
    };
    let broad = ValuationRequest {
        strategy: StrategyId::collection_floor(),
        ..request.clone()
    };
    let permit = hold(&data.reader).await;
    let jobs = async { tokio::join!(registry.mark(request.clone()), registry.mark(broad.clone())) };
    tokio::pin!(jobs);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut jobs)
            .await
            .is_err()
    );
    assert!(data.handle.status().await.unwrap().ready);
    drop(permit);
    let (missing, priced) = jobs.await;
    let error = missing.unwrap_err();
    assert_eq!(error.code(), ValuationErrorCode::NotEnoughData);
    assert_eq!(error.details().unwrap().strategy, StrategyId::model_floor());
    let priced = priced.unwrap();
    assert_eq!(priced.strategy, "collection_floor");
    assert_eq!(priced.total.as_str(), "6");
    assert!(Arc::ptr_eq(
        &priced,
        &registry.mark(broad.clone()).await.unwrap()
    ));
    let unknown = ValuationRequest {
        strategy: StrategyId::new("unknown").unwrap(),
        ..request.clone()
    };
    let error = registry.mark(unknown).await.unwrap_err();
    assert_eq!(error.code(), ValuationErrorCode::UnsupportedStrategy);
    assert_eq!(error.details().unwrap().strategy.as_str(), "unknown");
    // No eligible asks is an explicit reason, never a cached zero estimate.
    let mut exhausted = order("0");
    if let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(o)),
    })) = exhausted.change.as_mut()
    {
        o.quantity = "0".into();
    }
    writer
        .apply(w::Batch {
            cursor: "empty".into(),
            changes: vec![exhausted],
        })
        .unwrap();
    let error = registry.mark(broad.clone()).await.unwrap_err();
    assert_eq!(error.code(), ValuationErrorCode::NotEnoughData);
    let Some(crate::ValuationErrorContext::Portfolio { unpriced }) =
        &error.details().unwrap().context
    else {
        panic!("portfolio context required");
    };
    assert_eq!(unpriced[0].unpriced, Some(crate::UnpricedReason::NoListing));
    writer
        .apply(w::Batch {
            cursor: "recovered".into(),
            changes: vec![order("4")],
        })
        .unwrap();
    assert_eq!(
        registry.mark(broad.clone()).await.unwrap().total.as_str(),
        "8"
    );
    model.task.shutdown().await.unwrap();
    collection.task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

struct SnapshotProbe {
    source: ValuationSource,
    life: crate::Lifecycle,
    entered: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    release: Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>>,
}
impl Actor for SnapshotProbe {
    type Context = Context<Self>;
    fn started(&mut self, ctx: &mut Self::Context) {
        self.life.started(ctx);
    }
}
type ProbeMessage =
    crate::Evaluate<(), (std::thread::ThreadId, std::thread::ThreadId), crate::ValuationError>;
impl Handler<ProbeMessage> for SnapshotProbe {
    type Result = AtomicResponse<
        Self,
        Result<
            Arc<crate::Mark<(std::thread::ThreadId, std::thread::ThreadId)>>,
            crate::ValuationError,
        >,
    >;
    fn handle(&mut self, message: ProbeMessage, _: &mut Self::Context) -> Self::Result {
        let (_, request) = message.into_parts();
        let source = self.source.clone();
        let entered = self.entered.clone();
        let release = self.release.clone();
        let stop = self.life.stop_token();
        AtomicResponse::new(Box::pin(async move{
            let _request=request;
            tokio::select!{biased;
                _=stop.cancelled()=>Err(ValuationError::unavailable("Stopping")),
                _=_request.cancelled.cancelled()=>Err(ValuationError::unavailable("Cancelled")),
                result=source.prepare(move|snapshot|{
                    if let Some(entered)=entered.lock().unwrap().take(){let _=entered.send(());}
                    if let Some(release)=release.lock().unwrap().take(){release.recv().unwrap();}
                    Ok(crate::Prepared {stale:false, prepared_at:jiff::Timestamp::now(),revision:snapshot.revision()?,data:std::thread::current().id()})
                })=>result?,
            }
        }.into_actor(self).map(|result,actor,_|result.map(|prepared|Arc::new(crate::Mark::evaluated(actor.life.strategy(),crate::Prepared {stale:false, prepared_at:jiff::Timestamp::now(),revision:prepared.revision,data:(std::thread::current().id(),prepared.data)}))))))
    }
}
#[tokio::test]
async fn strategy_actors_share_arbiter_and_read_only_views_while_preparation_runs_elsewhere() {
    let (_directory, mut writer) = fixture();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let snapshot = data.reader.snapshot().unwrap();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let first = crate::Service::start_on(
        &data.arbiter(),
        "snapshot_probe",
        {
            let source = data.reader.clone();
            move |life| SnapshotProbe {
                source,
                life,
                entered: Arc::new(std::sync::Mutex::new(Some(entered))),
                release: Arc::new(std::sync::Mutex::new(Some(blocked))),
            }
        },
        Default::default(),
    )
    .await
    .unwrap();
    let second = crate::Service::start_on(
        &data.arbiter(),
        "snapshot_probe",
        {
            let source = data.reader.clone();
            move |life| SnapshotProbe {
                source,
                life,
                entered: Default::default(),
                release: Default::default(),
            }
        },
        Default::default(),
    )
    .await
    .unwrap();
    let handle = first.handle.clone();
    let request = tokio::spawn(async move { handle.evaluate(()).await });
    started.await.unwrap();
    writer
        .apply(w::Batch {
            cursor: "newer".into(),
            changes: vec![order("7")],
        })
        .unwrap();
    assert_eq!(snapshot.revision().unwrap(), "c0");
    assert_eq!(data.reader.snapshot().unwrap().revision().unwrap(), "newer");
    assert!(
        tokio::time::timeout(Duration::from_secs(1), data.handle.status())
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    release.send(()).unwrap();
    let old = request.await.unwrap().unwrap();
    assert_eq!(old.revision, "c0");
    let current = second.handle.evaluate(()).await.unwrap();
    assert_eq!(current.revision, "newer");
    assert_eq!(old.data.0, current.data.0);
    assert_ne!(old.data.0, old.data.1);
    assert_ne!(current.data.0, current.data.1);
    first.task.shutdown().await.unwrap();
    second.task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
#[tokio::test]
async fn cancelled_strategy_preparation_retains_its_worker_until_shutdown_drains_it() {
    let (_directory, writer) = fixture();
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let strategy = crate::Service::start_on(
        &data.arbiter(),
        "snapshot_probe",
        {
            let source = data.reader.clone();
            move |life| SnapshotProbe {
                source,
                life,
                entered: Arc::new(std::sync::Mutex::new(Some(entered))),
                release: Arc::new(std::sync::Mutex::new(Some(blocked))),
            }
        },
        Default::default(),
    )
    .await
    .unwrap();
    let handle = strategy.handle.clone();
    let request = tokio::spawn(async move { handle.evaluate(()).await });
    started.await.unwrap();
    strategy.task.shutdown().await.unwrap();
    assert!(matches!(request.await.unwrap(), Err(crate::Error::Stopped)));
    let stop = CancellationToken::new();
    stop.cancel();
    let mut drain = tokio::spawn(data.task.run(stop));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut drain)
            .await
            .is_err()
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), drain)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[cfg(feature = "portfolio")]
async fn calculate(
    source: &ValuationSource,
    request: crate::ValuationRequest,
) -> Result<Arc<Mark<crate::Valuation>>, ValuationError> {
    use crate::*;
    let kind = if request.strategy == StrategyId::model_floor() {
        calculation::FloorKind::Model
    } else {
        calculation::FloorKind::Collection
    };
    let input = calculation::normalize(PortfolioPositions {
        positions: request.positions,
        currency_id: request.currency_id,
    })?;
    source
        .prepare(move |snapshot| {
            calculation::portfolio(snapshot, input, kind)
                .map(|(prepared, _)| Arc::new(Mark::evaluated(kind.id(), prepared)))
        })
        .await?
}
pub(crate) struct HeldWorker(Option<std::sync::mpsc::Sender<()>>);
impl Drop for HeldWorker {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}
pub(crate) async fn hold(source: &ValuationSource) -> HeldWorker {
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let source = source.clone();
    tokio::spawn(async move {
        source
            .prepare(move |_| {
                let _ = entered.send(());
                let _ = blocked.recv();
            })
            .await
    });
    started.await.unwrap();
    HeldWorker(Some(release))
}

#[cfg(feature = "portfolio")]
pub(crate) fn model_fixture() -> (tempfile::TempDir, ReplicaWriter) {
    let (dir, mut writer) = fixture();
    let predicate = |id: &str| w::AttributePredicate {
        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
            value: Some(w::attribute_value::Value::OptionId(id.into())),
        })),
    };
    let mut changes = vec![entity(w::entity::Entity::ItemType(w::ItemType {
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
                    ids: vec!["a".into(), "b".into()],
                }),
            },
        )]
        .into(),
    }))];
    for id in ["a", "b"] {
        changes.push(entity(w::entity::Entity::AttributeOption(
            w::AttributeOption {
                id: id.into(),
                name: id.to_uppercase(),
                attribute_id: "tg.model".into(),
                ..Default::default()
            },
        )));
        for i in 0..3 {
            let mut row = order(if id == "a" { "10" } else { "20" });
            let Some(w::change::Change::Upsert(w::Entity {
                entity: Some(w::entity::Entity::Order(value)),
            })) = &mut row.change
            else {
                unreachable!()
            };
            value.id = format!("{id}.{i}");
            value.attributes = [("tg.model".into(), predicate(id))].into();
            changes.push(row);
        }
    }
    // The broad fixture ask must not set this test's collection floor.
    let mut broad = order("30");
    let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(value)),
    })) = &mut broad.change
    else {
        unreachable!()
    };
    value.quantity = "0".into();
    changes.push(broad);
    writer
        .apply(w::Batch {
            cursor: "models".into(),
            changes,
        })
        .unwrap();
    (dir, writer)
}

#[cfg(feature = "portfolio")]
#[tokio::test]
async fn floors_cache_multiple_portfolios_and_only_invalidate_relevant_dependencies() {
    use crate::*;
    let (_dir, mut writer) = model_fixture();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let service = Service::start_on(
        &data.arbiter(),
        "model_floor",
        {
            let source = data.reader.clone();
            move |life| FloorActor::model(source, life)
        },
        Default::default(),
    )
    .await
    .unwrap();
    let input = |model: &str| PortfolioPositions {
        currency_id: "GRAM".into(),
        positions: vec![ModelPosition {
            model: ModelLookup::Catalog {
                collection: "gift".into(),
                model: model.to_string().try_into().unwrap(),
            },
            quantity: 1,
        }],
    };
    let first = service.handle.evaluate(input("a")).await.unwrap();
    let other = service.handle.evaluate(input("b")).await.unwrap();
    assert_eq!(other.total.as_str(), "20");
    assert!(Arc::ptr_eq(
        &first,
        &service.handle.evaluate(input("a")).await.unwrap()
    ));
    let mut unrelated = order("1");
    let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(value)),
    })) = &mut unrelated.change
    else {
        unreachable!()
    };
    value.id = "unrelated".into();
    value.book.as_mut().unwrap().item_type_id = "another".into();
    writer
        .apply(w::Batch {
            cursor: "unrelated".into(),
            changes: vec![
                unrelated,
                entity(w::entity::Entity::ItemType(w::ItemType {
                    id: "another".into(),
                    name: "Another".into(),
                    quantity_step: "1".into(),
                    ..Default::default()
                })),
                entity(w::entity::Entity::AttributeOption(w::AttributeOption {
                    id: "unrelated".into(),
                    attribute_id: "tg.model".into(),
                    name: "Unrelated".into(),
                    ..Default::default()
                })),
            ],
        })
        .unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &service.handle.evaluate(input("a")).await.unwrap()
    ));
    writer
        .apply(w::Batch {
            cursor: "renamed".into(),
            changes: vec![entity(w::entity::Entity::AttributeOption(
                w::AttributeOption {
                    id: "a".into(),
                    attribute_id: "tg.model".into(),
                    name: "Renamed".into(),
                    ..Default::default()
                },
            ))],
        })
        .unwrap();
    let renamed = service.handle.evaluate(input("a")).await.unwrap();
    assert!(!Arc::ptr_eq(&first, &renamed));
    assert_eq!(renamed.rows[0].model, "Renamed");
    let mut changed = order("1");
    let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(value)),
    })) = &mut changed.change
    else {
        unreachable!()
    };
    value.id = "a.0".into();
    value.attributes = [(
        "tg.model".into(),
        w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                value: Some(w::attribute_value::Value::OptionId("a".into())),
            })),
        },
    )]
    .into();
    writer
        .apply(w::Batch {
            cursor: "cheaper".into(),
            changes: vec![changed],
        })
        .unwrap();
    let cheaper = service.handle.evaluate(input("a")).await.unwrap();
    assert_eq!(cheaper.total.as_str(), "1");
    assert_eq!(cheaper.revision, "cheaper");
    writer.mark(market_replica::SyncState::Stale);
    let stale = service.handle.evaluate(input("a")).await.unwrap();
    assert_eq!(stale.sync.state, market_gifts::SyncState::Stale);
    service.task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[cfg(feature = "portfolio")]
#[tokio::test]
async fn catalog_resolution_and_floor_calculation_share_the_pinned_view() {
    use crate::*;
    let (_dir, mut writer) = model_fixture();
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let pinned = data.reader.snapshot().unwrap();
    let input = PortfolioPositions {
        positions: vec![ModelPosition {
            model: ModelLookup::named("Gift", "A").unwrap(),
            quantity: 2,
        }],
        currency_id: "GRAM".into(),
    };
    writer
        .apply(w::Batch {
            cursor: "renamed".into(),
            changes: vec![entity(w::entity::Entity::AttributeOption(
                w::AttributeOption {
                    id: "a".into(),
                    attribute_id: "tg.model".into(),
                    name: "Renamed".into(),
                    ..Default::default()
                },
            ))],
        })
        .unwrap();
    let (old, dependencies) =
        calculation::portfolio(pinned, input.clone(), calculation::FloorKind::Model).unwrap();
    assert_eq!(old.data.total.as_str(), "20");
    assert_eq!(old.data.rows[0].model, "A");
    assert_eq!(old.revision, "models");
    assert!(!data.reader.is_current(&dependencies).await);
    let (new, _) = calculation::portfolio(
        data.reader.snapshot().unwrap(),
        input,
        calculation::FloorKind::Model,
    )
    .unwrap();
    assert_eq!(new.data.total.as_str(), "0");
    assert_eq!(
        new.data.rows[0].unpriced,
        Some(UnpricedReason::UnknownModel)
    );
    assert_eq!(new.revision, "renamed");
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[cfg(feature = "portfolio")]
#[tokio::test]
async fn ask_projection_skips_unsupported_and_expired_rows_and_preserves_singleton_traits() {
    let (_dir, mut writer) = model_fixture();
    let make = |id: &str, price: &str| {
        let mut row = order(price);
        let Some(w::change::Change::Upsert(w::Entity {
            entity: Some(w::entity::Entity::Order(value)),
        })) = &mut row.change
        else {
            unreachable!()
        };
        value.id = id.into();
        value.attributes = [(
            "tg.model".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::OneOf(
                    w::AttributeValues {
                        values: vec![w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId("a".into())),
                        }],
                    },
                )),
            },
        )]
        .into();
        row
    };
    let mut unsupported = make("unsupported", "1");
    let mut expired = make("expired", "2");
    if let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(value)),
    })) = &mut unsupported.change
    {
        value
            .attributes
            .insert("unsupported".into(), w::AttributePredicate::default());
    }
    if let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(value)),
    })) = &mut expired.change
    {
        value.expires_at_ms = Some(10);
    }
    writer
        .apply(w::Batch {
            cursor: "eligibility".into(),
            changes: vec![unsupported, expired, make("singleton", "3")],
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let scope = crate::source::BookScope {
        collection: "gift".into(),
        currency: "GRAM".into(),
    };
    let page = data
        .reader
        .snapshot()
        .unwrap()
        .asks(&scope, 10, String::new())
        .unwrap();
    assert_eq!(page.data[0].price.as_str(), "3");
    assert_eq!(page.data[0].model_id.as_deref(), Some("a"));
    assert_eq!(page.data.len(), 7);
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[cfg(feature = "portfolio")]
#[tokio::test]
async fn name_lookup_cache_detects_new_ambiguity_without_invalidating_catalog_ids() {
    use crate::*;
    let (_dir, mut writer) = model_fixture();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let service = Service::start_on(
        &data.arbiter(),
        "model_floor",
        {
            let source = data.reader.clone();
            move |life| FloorActor::model(source, life)
        },
        Default::default(),
    )
    .await
    .unwrap();
    let input = |model| PortfolioPositions {
        currency_id: "GRAM".into(),
        positions: vec![ModelPosition { model, quantity: 1 }],
    };
    let catalog = input(ModelLookup::Catalog {
        collection: "gift".into(),
        model: "a".into(),
    });
    let named = input(ModelLookup::named("Gift", "A").unwrap());
    let original = service.handle.evaluate(catalog.clone()).await.unwrap();
    assert_eq!(
        service
            .handle
            .evaluate(named.clone())
            .await
            .unwrap()
            .total
            .as_str(),
        "10"
    );
    writer
        .apply(w::Batch {
            cursor: "ambiguous".into(),
            changes: vec![entity(w::entity::Entity::ItemType(w::ItemType {
                id: "duplicate".into(),
                name: "Gift".into(),
                quantity_step: "1".into(),
                aliases: vec![w::ExternalId {
                    namespace: "tg".into(),
                    id: "100".into(),
                }],
                ..Default::default()
            }))],
        })
        .unwrap();
    let Error::Strategy(error) = service.handle.evaluate(named).await.unwrap_err() else {
        panic!("expected ambiguous lookup");
    };
    let Some(ValuationErrorContext::Portfolio { unpriced }) = &error.details().unwrap().context
    else {
        panic!("expected lookup evidence");
    };
    assert_eq!(
        unpriced[0].unpriced,
        Some(UnpricedReason::AmbiguousCollection)
    );
    assert!(Arc::ptr_eq(
        &original,
        &service.handle.evaluate(catalog).await.unwrap()
    ));
    service.task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
