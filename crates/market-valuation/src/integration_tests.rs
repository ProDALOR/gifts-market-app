use crate::*;
use actix::prelude::*;
use market_actors::{MarketDataService, MarketReader};
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
    let value = calculate(
        &service.reader,
        ValuationRequest {
            strategy: market_gifts::valuation::StrategyId::model_floor(),
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
            strategy: market_gifts::valuation::StrategyId::model_floor(),
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
                strategy: market_gifts::valuation::StrategyId::model_floor(),
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

#[tokio::test]
async fn strategy_registry_shares_read_budget_and_preserves_typed_failures() {
    use crate::ValuationErrorCode;
    use crate::{FloorActor, ValuationHandle};
    use market_gifts::valuation::{ModelLookup, ModelPosition, StrategyId, ValuationRequest};
    let (_directory, mut writer) = fixture();
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let model =
        crate::Service::start_on(&data, "model_floor", FloorActor::model, Default::default())
            .await
            .unwrap();
    let collection = crate::Service::start_on(
        &data,
        "collection_floor",
        FloorActor::collection,
        Default::default(),
    )
    .await
    .unwrap();
    assert!(ValuationHandle::new([] as [crate::FloorStrategyHandle; 0]).is_err());
    assert!(ValuationHandle::new([model.handle.clone(), model.handle.clone()]).is_err());
    let registry = ValuationHandle::new([model.handle.clone(), collection.handle.clone()]).unwrap();
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
    let Some(market_gifts::valuation::ValuationErrorContext::Portfolio { unpriced }) =
        &error.details().unwrap().context
    else {
        panic!("portfolio context required");
    };
    assert_eq!(
        unpriced[0].unpriced,
        Some(market_gifts::valuation::UnpricedReason::NoListing)
    );
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

#[tokio::test]
async fn typed_multipliers_enforce_evidence_and_recover_without_stale_marks() {
    use crate::*;
    use market_gifts::valuation::*;
    let (_directory, mut writer) = fixture();
    let option = |id: &str, attribute: &str| {
        entity(w::entity::Entity::AttributeOption(w::AttributeOption {
            id: id.into(),
            attribute_id: attribute.into(),
            name: id.into(),
            ..Default::default()
        }))
    };
    let ask = |id: &str, price: &str, model: &str, backdrop: &str| {
        let mut row = w::Order {
            id: id.into(),
            book: Some(w::BookKey {
                venue_id: "venue".into(),
                item_type_id: "gift".into(),
                currency_id: "GRAM".into(),
            }),
            mode: w::OrderMode::Ask as i32,
            price: price.into(),
            quantity: "1".into(),
            ..Default::default()
        };
        for (attribute, value) in [("tg.model", model), ("tg.backdrop", backdrop)] {
            row.attributes.insert(
                attribute.into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId(value.into())),
                    })),
                },
            );
        }
        row
    };
    let mut item = w::ItemType {
        id: "gift".into(),
        name: "Gift".into(),
        quantity_step: "1".into(),
        aliases: vec![w::ExternalId {
            namespace: "tg".into(),
            id: "99".into(),
        }],
        ..Default::default()
    };
    for (attribute, ids) in [
        ("tg.model", vec!["m", "other"]),
        ("tg.backdrop", vec!["b", "base"]),
    ] {
        item.attributes.insert(
            attribute.into(),
            w::AttributeRule {
                required: true,
                allowed_values: Some(w::AllowedAttributeValues {
                    ids: ids.into_iter().map(str::to_owned).collect(),
                }),
            },
        );
    }
    let mut changes = vec![
        entity(w::entity::Entity::ItemType(item)),
        option("m", "tg.model"),
        option("other", "tg.model"),
        option("b", "tg.backdrop"),
        option("base", "tg.backdrop"),
    ];
    for (id, price, model, backdrop) in [
        ("order", "2", "other", "base"),
        ("base1", "3", "other", "base"),
        ("base2", "4", "other", "base"),
        ("trait0", "6", "m", "b"),
        ("trait1", "7", "m", "b"),
        ("trait2", "8", "m", "b"),
    ] {
        changes.push(entity(w::entity::Entity::Order(ask(
            id, price, model, backdrop,
        ))));
    }
    for (id, kind) in [
        ("expired", 0),
        ("bid", 1),
        ("empty", 2),
        ("zero", 3),
        ("unsupported", 4),
        ("other-currency", 5),
    ] {
        let mut row = ask(id, "0.1", "m", "b");
        match kind {
            0 => row.expires_at_ms = Some(1),
            1 => row.mode = w::OrderMode::Bid as i32,
            2 => row.quantity = "0".into(),
            3 => row.price = "0".into(),
            4 => {
                row.attributes.insert(
                    "unknown".into(),
                    w::AttributePredicate {
                        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                            value: Some(w::attribute_value::Value::Boolean(true)),
                        })),
                    },
                );
            }
            _ => row.book.as_mut().unwrap().currency_id = "XTR".into(),
        };
        changes.push(entity(w::entity::Entity::Order(row)));
    }
    writer
        .apply(w::Batch {
            cursor: "ratios".into(),
            changes,
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let model = crate::Service::start_on(
        &data,
        "model_multiplier",
        ModelMultiplierActor::new,
        Default::default(),
    )
    .await
    .unwrap();
    let backdrop = crate::Service::start_on(
        &data,
        "backdrop_multiplier",
        BackdropMultiplierActor::new,
        Default::default(),
    )
    .await
    .unwrap();
    let registry = ValuationHandle::new([
        RegisteredStrategy::from(model.handle.clone()),
        RegisteredStrategy::from(backdrop.handle.clone()),
    ])
    .unwrap();
    let request = ModelMultiplierRequest {
        collection_id: "gift".into(),
        model_id: "m".into(),
        currency_id: "GRAM".into(),
    };
    let backdrop_request = BackdropMultiplierRequest {
        collection_id: "gift".into(),
        backdrop_id: "b".into(),
        currency_id: "GRAM".into(),
    };
    let (a, b) = tokio::join!(
        registry.query(ValuationQuery::ModelMultiplier(request.clone())),
        registry.query(ValuationQuery::BackdropMultiplier(backdrop_request.clone()))
    );
    let ValuationResult::ModelMultiplier(a) = a.unwrap() else {
        panic!("model result");
    };
    let ValuationResult::BackdropMultiplier(b) = b.unwrap() else {
        panic!("backdrop result");
    };
    assert_eq!(a.factor.as_str(), "3");
    assert_eq!(b.factor.as_str(), "3");
    assert_eq!(a.evidence.collection.samples, 6);
    assert_eq!(a.evidence.selected_trait.samples, 3);
    assert_eq!(a.revision, "ratios");
    assert_eq!(a.strategy, "model_multiplier");
    assert!(Arc::ptr_eq(
        &a,
        &model.handle.evaluate(request.clone()).await.unwrap()
    ));
    let mut unrelated = ask("unrelated", "1", "other", "base");
    unrelated.book.as_mut().unwrap().item_type_id = "another-gift".into();
    writer
        .apply(w::Batch {
            cursor: "unrelated-change".into(),
            changes: vec![entity(w::entity::Entity::Order(unrelated))],
        })
        .unwrap();
    assert!(Arc::ptr_eq(
        &a,
        &model.handle.evaluate(request.clone()).await.unwrap()
    ));
    // Another model uses this actor's prepared collection state without acquiring a read worker.
    let occupied = hold(&data.reader).await;
    let another = ModelMultiplierRequest {
        model_id: "other".into(),
        ..request.clone()
    };
    let cached = tokio::time::timeout(Duration::from_secs(1), model.handle.evaluate(another))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cached.factor.as_str(), "1");
    assert_eq!(cached.evaluated_at, a.evaluated_at);
    drop(occupied);
    // The requested trait is beyond the first 500 ascending-price rows.
    writer
        .apply(w::Batch {
            cursor: "paginated-ratio".into(),
            changes: (0..501)
                .map(|i| {
                    entity(w::entity::Entity::Order(ask(
                        &format!("bulk-{i}"),
                        "5",
                        "other",
                        "base",
                    )))
                })
                .collect(),
        })
        .unwrap();
    let ValuationResult::BackdropMultiplier(paginated) = registry
        .query(ValuationQuery::BackdropMultiplier(backdrop_request))
        .await
        .unwrap()
    else {
        panic!("backdrop result");
    };
    assert_eq!(paginated.evidence.collection.samples, 507);
    assert_eq!(paginated.evidence.selected_trait.samples, 3);
    assert_eq!(paginated.factor.as_str(), "3");
    assert_eq!(paginated.revision, "paginated-ratio");
    let wrong = ModelMultiplierRequest {
        model_id: "b".into(),
        ..request.clone()
    };
    assert_eq!(
        registry
            .query(ValuationQuery::ModelMultiplier(wrong))
            .await
            .unwrap_err()
            .code(),
        crate::ValuationErrorCode::InvalidQuery
    );
    let mut exhausted = ask("trait0", "6", "m", "b");
    exhausted.quantity = "0".into();
    writer
        .apply(w::Batch {
            cursor: "insufficient".into(),
            changes: vec![entity(w::entity::Entity::Order(exhausted))],
        })
        .unwrap();
    let error = registry
        .query(ValuationQuery::ModelMultiplier(request.clone()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), crate::ValuationErrorCode::NotEnoughData);
    let details = error.details().unwrap();
    assert_eq!(details.available_samples, Some(2));
    assert_eq!(details.required_samples, Some(3));
    assert_eq!(details.revision.as_deref(), Some("insufficient"));
    assert!(matches!(
        details.context,
        Some(ValuationErrorContext::Multiplier { .. })
    ));
    writer
        .apply(w::Batch {
            cursor: "ratio-recovered".into(),
            changes: vec![
                entity(w::entity::Entity::Order(ask("trait0", "6", "m", "b"))),
                entity(w::entity::Entity::Order(ask("order", "1", "other", "base"))),
            ],
        })
        .unwrap();
    let ValuationResult::ModelMultiplier(fresh) = registry
        .query(ValuationQuery::ModelMultiplier(request))
        .await
        .unwrap()
    else {
        panic!("model result");
    };
    assert_eq!(fresh.factor.as_str(), "6");
    assert_eq!(fresh.revision, "ratio-recovered");
    assert!(!Arc::ptr_eq(&a, &fresh));
    model.task.shutdown().await.unwrap();
    backdrop.task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

struct SnapshotProbe {
    source: MarketReader,
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
                    Ok(crate::Prepared { prepared_at:jiff::Timestamp::now(),revision:snapshot.revision()?,data:std::thread::current().id()})
                })=>result?,
            }
        }.into_actor(self).map(|result,actor,_|result.map(|prepared|Arc::new(crate::Mark::evaluated(actor.life.strategy(),crate::Prepared { prepared_at:jiff::Timestamp::now(),revision:prepared.revision,data:(std::thread::current().id(),prepared.data)}))))))
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
        &data,
        "snapshot_probe",
        move |source, life| SnapshotProbe {
            source,
            life,
            entered: Arc::new(std::sync::Mutex::new(Some(entered))),
            release: Arc::new(std::sync::Mutex::new(Some(blocked))),
        },
        Default::default(),
    )
    .await
    .unwrap();
    let second = crate::Service::start_on(
        &data,
        "snapshot_probe",
        |source, life| SnapshotProbe {
            source,
            life,
            entered: Default::default(),
            release: Default::default(),
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
        &data,
        "snapshot_probe",
        move |source, life| SnapshotProbe {
            source,
            life,
            entered: Arc::new(std::sync::Mutex::new(Some(entered))),
            release: Arc::new(std::sync::Mutex::new(Some(blocked))),
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

async fn calculate(
    source: &MarketReader,
    request: market_gifts::valuation::ValuationRequest,
) -> Result<Arc<Mark<market_gifts::valuation::Valuation>>, ValuationError> {
    use market_gifts::valuation::*;
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
pub(crate) async fn hold(source: &MarketReader) -> HeldWorker {
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

#[tokio::test]
async fn floors_cache_multiple_portfolios_and_only_invalidate_relevant_dependencies() {
    use market_gifts::valuation::*;
    let (_dir, mut writer) = model_fixture();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let service = Service::start_on(&data, "model_floor", FloorActor::model, Default::default())
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

#[tokio::test]
async fn catalog_resolution_and_floor_calculation_share_the_pinned_view() {
    use market_gifts::valuation::*;
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
    let scope = market_actors::BookScope {
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

#[tokio::test]
async fn name_lookup_cache_detects_new_ambiguity_without_invalidating_catalog_ids() {
    use market_gifts::valuation::*;
    let (_dir, mut writer) = model_fixture();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let service = Service::start_on(&data, "model_floor", FloorActor::model, Default::default())
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

#[tokio::test]
async fn model_multiplier_ignores_unrestricted_backdrops_and_a_large_unrelated_catalog() {
    use market_gifts::valuation::*;
    let (_dir, mut writer) = model_fixture();
    let mut changes = (0..50_001)
        .map(|i| {
            entity(w::entity::Entity::AttributeOption(w::AttributeOption {
                id: format!("backdrop.{i}"),
                attribute_id: "tg.backdrop".into(),
                name: format!("Backdrop {i}"),
                ..Default::default()
            }))
        })
        .collect::<Vec<_>>();
    changes.push(entity(w::entity::Entity::ItemType(w::ItemType {
        id: "gift".into(),
        name: "Gift".into(),
        quantity_step: "1".into(),
        aliases: vec![w::ExternalId {
            namespace: "tg".into(),
            id: "99".into(),
        }],
        attributes: [
            (
                "tg.model".into(),
                w::AttributeRule {
                    required: true,
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec!["a".into(), "b".into()],
                    }),
                },
            ),
            (
                "tg.backdrop".into(),
                w::AttributeRule {
                    required: false,
                    allowed_values: None,
                },
            ),
        ]
        .into(),
    })));
    writer
        .apply(w::Batch {
            cursor: "large-catalog".into(),
            changes,
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let service = Service::start_on(
        &data,
        "model_multiplier",
        ModelMultiplierActor::new,
        Default::default(),
    )
    .await
    .unwrap();
    let request = ModelMultiplierRequest {
        collection_id: "gift".into(),
        model_id: "b".into(),
        currency_id: "GRAM".into(),
    };
    let original = service.handle.evaluate(request.clone()).await.unwrap();
    assert_eq!(original.factor.as_str(), "2");
    writer
        .apply(w::Batch {
            cursor: "palette".into(),
            changes: vec![entity(w::entity::Entity::AttributeOption(
                w::AttributeOption {
                    id: "backdrop.0".into(),
                    attribute_id: "tg.backdrop".into(),
                    name: "Changed backdrop".into(),
                    ..Default::default()
                },
            ))],
        })
        .unwrap();
    assert!(Arc::ptr_eq(
        &original,
        &service.handle.evaluate(request).await.unwrap()
    ));
    service.task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
