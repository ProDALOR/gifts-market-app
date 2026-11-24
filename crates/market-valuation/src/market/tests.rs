use super::*;
use crate::{
    gift_floor::tests::{ask, book},
    integration_tests::entity,
    test_support::MarketDataService,
};
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn request(target: ValuationTarget) -> ValuationQuery {
    ValuationQuery {
        target,
        currency_id: "GRAM".into(),
        strategies: ["ask_floor", "execution_mean", "execution_median"]
            .map(|id| StrategyId::new(id).unwrap())
            .into(),
        period: Some(Period {
            from: Some("2026-01-01T00:00:00Z".parse().unwrap()),
            until: Some("2026-02-01T00:00:00Z".parse().unwrap()),
        }),
    }
}
pub(super) fn execution(i: usize, price: &str, time: &str) -> w::Change {
    let mut row = ask(
        &format!("execution{i}"),
        "m",
        "b",
        price,
        3000 + i as i64,
        None,
    );
    let Some(w::change::Change::Upsert(entity)) = &mut row.change else {
        unreachable!()
    };
    let Some(w::entity::Entity::Order(order)) = &mut entity.entity else {
        unreachable!()
    };
    order.mode = w::OrderMode::Executed as i32;
    order.executed_at_ms = Some(time.parse::<jiff::Timestamp>().unwrap().as_millisecond());
    row
}
#[tokio::test]
async fn targets_share_floor_statistics_and_execution_periods_without_requiring_an_individual_gift()
{
    let (_dir, mut writer) = book();
    writer
        .apply(w::Batch {
            cursor: "history".into(),
            changes: vec![
                execution(0, "5", "2026-01-01T00:00:00Z"),
                execution(1, "15", "2026-01-15T00:00:00Z"),
                execution(2, "100", "2026-01-31T00:00:00Z"),
                execution(3, "10000", "2026-02-01T00:00:00Z"),
            ],
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    for (target, floor) in [
        (
            ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: market_gifts::GiftCriteria::default(),
            },
            "3",
        ),
        (
            ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: market_gifts::GiftCriteria {
                    models: vec!["m".into()],
                    ..Default::default()
                },
            },
            "10",
        ),
        (
            ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: market_gifts::GiftCriteria {
                    backdrops: vec!["b".into()],
                    ..Default::default()
                },
            },
            "11",
        ),
        (
            ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: market_gifts::GiftCriteria {
                    models: vec!["m".into()],
                    backdrops: vec!["b".into()],
                    ..Default::default()
                },
            },
            "100",
        ),
    ] {
        let report = handle.query(request(target)).await.unwrap();
        let [
            StrategyOutcome::Value(StrategyValuation::AskFloor(ask)),
            StrategyOutcome::Value(StrategyValuation::ExecutionMean(mean)),
            StrategyOutcome::Value(StrategyValuation::ExecutionMedian(median)),
        ] = report.results.as_slice()
        else {
            panic!("unexpected outcomes: {:?}", report.results)
        };
        assert_eq!(ask.price.value().unwrap().as_str(), floor);
        assert_eq!(mean.price.value().unwrap().as_str(), "40");
        assert_eq!(median.price.value().unwrap().as_str(), "15");
        assert_eq!(mean.evidence.executions, 3);
        assert_eq!(mean.evidence.gifts, 3);
        assert_eq!(
            mean.evidence.first_execution,
            request(ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: market_gifts::GiftCriteria::default()
            })
            .period
            .unwrap()
            .from
        );
        assert!(!report.stale);
    }
    let mut mixed = request(ValuationTarget::Selection {
        collection_id: "gift".into(),
        criteria: market_gifts::GiftCriteria::default(),
    });
    mixed
        .strategies
        .push(StrategyId::new("gift_floor").unwrap());
    let report = handle.query(mixed).await.unwrap();
    assert!(
        matches!(&report.results[3],StrategyOutcome::Error{error,..} if error.code==StrategyFailureCode::UnsupportedTarget)
    );
    assert!(matches!(&report.results[0], StrategyOutcome::Value(_)));
    // Metadata membership errors are not treated as empty evidence.
    let invalid = request(ValuationTarget::Selection {
        collection_id: "gift".into(),
        criteria: market_gifts::GiftCriteria {
            models: vec!["unknown".into()],
            ..Default::default()
        },
    });
    assert!(handle.query(invalid).await.is_err());
    assert!(
        handle
            .query(request(ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: market_gifts::GiftCriteria {
                    models: vec!["b".into()],
                    ..Default::default()
                }
            }))
            .await
            .is_err()
    );
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
#[tokio::test]
async fn sparse_history_preserves_ask_results_and_individual_gifts_can_use_their_own_execution_history()
 {
    let (_dir, mut writer) = book();
    let mut executions = vec![];
    for i in 0..3 {
        let mut row = execution(i, "20", "2026-01-10T00:00:00Z");
        let Some(w::change::Change::Upsert(entity)) = &mut row.change else {
            unreachable!()
        };
        let Some(w::entity::Entity::Order(order)) = &mut entity.entity else {
            unreachable!()
        };
        order.item_id = Some("gift-pair0".into());
        order.attributes.insert(
            "tg.number".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::Integer(123)),
                })),
            },
        );
        executions.push(row);
    }
    writer
        .apply(w::Batch {
            changes: executions,
            cursor: "same-gift-history".into(),
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let collection = handle
        .query(request(ValuationTarget::Selection {
            collection_id: "gift".into(),
            criteria: market_gifts::GiftCriteria::default(),
        }))
        .await
        .unwrap();
    let StrategyOutcome::Value(StrategyValuation::ExecutionMean(mean)) = &collection.results[1]
    else {
        panic!()
    };
    assert!(mean.price.value().is_none());
    assert_eq!(mean.evidence.gifts, 1);
    let gift = handle
        .query(request(ValuationTarget::Gift {
            gift: GiftLookup::Catalog {
                collection_id: "gift".into(),
                number: GiftNumber::new(123).unwrap(),
            },
        }))
        .await
        .unwrap();
    let StrategyOutcome::Value(StrategyValuation::ExecutionMean(mean)) = &gift.results[1] else {
        panic!()
    };
    assert_eq!(mean.price.value().unwrap().as_str(), "20");
    assert_eq!(mean.evidence.minimum_gifts, 1);
    assert!(
        matches!(&collection.results[0],StrategyOutcome::Value(StrategyValuation::AskFloor(value)) if value.price.value().is_some())
    );
    writer.mark(market_replica::SyncState::Stale);
    assert!(
        handle
            .query(request(ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: market_gifts::GiftCriteria::default()
            }))
            .await
            .unwrap()
            .stale
    );
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
#[test]
fn strategies_and_execution_periods_are_explicit_and_validated() {
    let mut q = request(ValuationTarget::Selection {
        collection_id: "gift".into(),
        criteria: market_gifts::GiftCriteria::default(),
    });
    assert!(q.validate().is_ok());
    q.period = None;
    assert!(q.validate().is_err());
    q.strategies.clear();
    assert!(q.validate().is_err());
    q.strategies = vec![StrategyId::new("ask_floor").unwrap()];
    assert!(q.validate().is_ok());
    q.strategies.push(q.strategies[0].clone());
    assert!(q.validate().is_err());
    q.strategies = vec![StrategyId::new("unknown").unwrap()];
    assert!(q.validate().is_err());
}

#[test]
fn preparation_preserves_strategy_order_and_shares_one_checked_period() {
    let mut query = request(ValuationTarget::Selection {
        collection_id: "gift".into(),
        criteria: market_gifts::GiftCriteria::default(),
    });
    query.strategies.reverse();
    let ids = query.strategies.clone();
    let expected_period = query.period.clone().unwrap();
    let prepared = query.prepare().unwrap();
    assert_eq!(prepared.strategies.len(), ids.len());
    assert_eq!(prepared.period.unwrap().as_period(), expected_period);
    let target = Arc::new(ResolvedTarget::Selection {
        collection_id: "gift".into(),
        criteria: market_gifts::GiftCriteria::default(),
    });
    for (entry, id) in prepared.strategies.iter().zip(&ids) {
        assert_eq!(entry.metadata().id, id.as_str());
        assert!(std::ptr::eq(
            *entry,
            crate::strategy::registration(id.as_str()).unwrap(),
        ));
        if entry.metadata().input.requires_period() {
            let input = ExecutionInput::resolve(target.clone(), prepared.period).unwrap();
            assert_eq!(input.period().as_period(), expected_period);
        }
    }
}

#[test]
fn preparation_rejects_periods_unused_by_all_selected_strategies() {
    let mut query = request(ValuationTarget::Selection {
        collection_id: "gift".into(),
        criteria: Default::default(),
    });
    query.strategies = vec![StrategyId::new("ask_floor").unwrap()];
    assert!(query.clone().prepare().is_err());
    query.period.as_mut().unwrap().until = None;
    assert!(query.clone().prepare().is_err());
    query.period = None;
    assert!(query.prepare().is_ok());
}

#[tokio::test]
async fn bid_criteria_value_the_joint_selection_including_symbols_and_numbers() {
    let (_dir, mut writer) = book();
    let mut changes = vec![
        entity(w::entity::Entity::Attribute(w::Attribute {
            id: "tg.pattern".into(),
            name: "Symbol".into(),
            value_type: w::AttributeType::Enum as i32,
            ..Default::default()
        })),
        entity(w::entity::Entity::AttributeOption(w::AttributeOption {
            id: "symbol".into(),
            attribute_id: "tg.pattern".into(),
            name: "Symbol".into(),
            ..Default::default()
        })),
    ];
    let mut row = ask("selected", "m", "b", "120", 555, None);
    if let Some(w::change::Change::Upsert(w::Entity {
        entity: Some(w::entity::Entity::Order(order)),
    })) = &mut row.change
    {
        order.attributes.insert(
            "tg.pattern".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::OptionId("symbol".into())),
                })),
            },
        );
    }
    changes.push(row);
    writer
        .apply(w::Batch {
            cursor: "joint".into(),
            changes,
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    for (criteria, expected) in [
        (GiftCriteria::default(), Some("3")),
        (
            GiftCriteria {
                models: vec!["m".into()],
                backdrops: vec!["b".into()],
                ..Default::default()
            },
            Some("100"),
        ),
        (
            GiftCriteria {
                models: vec!["m".into()],
                backdrops: vec!["b".into()],
                patterns: vec!["symbol".into()],
                number: Some(GiftNumberRange {
                    min: Some(GiftNumber::new(500).unwrap()),
                    max: Some(GiftNumber::new(600).unwrap()),
                }),
            },
            Some("120"),
        ),
        (
            GiftCriteria {
                number: Some(GiftNumberRange {
                    min: Some(GiftNumber::new(999_999).unwrap()),
                    max: None,
                }),
                ..Default::default()
            },
            None,
        ),
    ] {
        let mut query = request(ValuationTarget::Selection {
            collection_id: "gift".into(),
            criteria,
        });
        query.strategies = vec![StrategyId::new("ask_floor").unwrap()];
        query.period = None;
        let result = handle.query(query).await.unwrap();
        let StrategyOutcome::Value(StrategyValuation::AskFloor(value)) = &result.results[0] else {
            panic!("{:?}", result.results)
        };
        assert_eq!(value.price.value().map(Amount::as_str), expected);
    }
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn every_registered_strategy_runs_through_its_typed_input_for_the_same_gift() {
    let (_dir, writer) = book();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let ids = crate::strategies_for(ValuationTargetKind::Gift, true)
        .map(|metadata| StrategyId::new(metadata.id).unwrap())
        .collect::<Vec<_>>();
    let mut query = request(ValuationTarget::Gift {
        gift: GiftLookup::Catalog {
            collection_id: "gift".into(),
            number: GiftNumber::new(123).unwrap(),
        },
    });
    query.strategies = ids.clone();
    let report = handle.query(query).await.unwrap();
    assert_eq!(report.results.len(), ids.len());
    for (result, id) in report.results.iter().zip(ids) {
        assert_eq!(result.strategy(), id.as_str());
        assert_eq!(
            serde_json::to_value(result).unwrap()["strategy"],
            id.as_str()
        );
        assert!(matches!(result, StrategyOutcome::Value(_)));
        let wire = serde_json::to_value(result).unwrap();
        let metadata = crate::strategy(result.strategy()).unwrap();
        for field in metadata.display.fields {
            let data = &wire["data"];
            let metric = field.path.map_or(&data[field.id], |path| {
                data.pointer(path).expect("declared metric path")
            });
            assert!(
                metric.is_object(),
                "{} missing metric {}",
                metadata.id,
                field.id
            );
            assert_ne!(metric.get("value").is_some(), metric.get("error").is_some());
        }
        for evidence in metadata.display.evidence {
            assert!(
                wire["data"].pointer(evidence.path).is_some(),
                "{} missing evidence {}",
                metadata.id,
                evidence.path
            );
        }
        assert_eq!(
            result.display(&report.currency_id).fields.len(),
            metadata.display.fields.len()
        );
    }
    let StrategyOutcome::Value(StrategyValuation::AskFloor(ask)) = &report.results[0] else {
        panic!()
    };
    assert_eq!(ask.price.value().unwrap().as_str(), "100");
    let StrategyOutcome::Value(StrategyValuation::GiftFloor(gift)) = &report
        .results
        .iter()
        .find(|row| row.strategy() == "gift_floor")
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(gift.final_mark.value().unwrap().as_str(), "18");
    // The fixture's gift has no symbol; the floor formula does not require one.
    assert!(report.target.gift().unwrap().traits.pattern.is_none());
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}

#[test]
fn computation_failures_exclude_storage_retry_codes_and_internal_messages() {
    for error in [
        tonic::Status::failed_precondition("replica checkpoint changed"),
        tonic::Status::unavailable("internal reader failure"),
    ] {
        let failure = StrategyFailure::from(ValuationError::from(error));
        assert_eq!(failure.code, StrategyFailureCode::Unavailable);
        assert_eq!(failure.message, "Valuation data is temporarily unavailable");
    }
    let failure = StrategyFailure::from(ValuationError::unsupported_target("Requires a gift"));
    assert_eq!(failure.code, StrategyFailureCode::UnsupportedTarget);
    assert_eq!(failure.message, "Requires a gift");
}

#[test]
fn shared_floor_values_flatten_count_without_legacy_wrappers_or_confidence_gates() {
    let floor = FloorValuation {
        price: ValuationMetric::Value {
            value: Amount::new("10.800000001").unwrap(),
        },
        count: 0,
    };
    let value = serde_json::to_value(&floor).unwrap();
    assert_eq!(value, serde_json::json!({"value":"10.800000001","count":0}));
    assert_eq!(
        serde_json::from_value::<FloorValuation>(value)
            .unwrap()
            .value(),
        floor.value()
    );
    let absent = FloorValuation {
        price: super::unavailable(
            ValuationIssueCode::NotEnoughData,
            "No matching asks",
            None,
            None,
        ),
        count: 0,
    };
    let value = serde_json::to_value(&absent).unwrap();
    assert_eq!(value["error"]["code"], "not_enough_data");
    assert_eq!(value["count"], 0);
    for key in ["price", "evidence", "gifts", "listings"] {
        assert!(value.get(key).is_none());
    }
    assert!(
        serde_json::from_value::<FloorValuation>(
            serde_json::json!({"price":{"value":"10"},"evidence":{"gifts":1,"listings":2}})
        )
        .is_err()
    );
}
