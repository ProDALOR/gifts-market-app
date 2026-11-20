use super::*;
use crate::{
    gift_floor::tests::{ask, book},
    integration_tests::entity,
    test_support::MarketDataService,
};
use market_gifts::*;
use market_hub_protocol::wire as w;
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
fn execution(i: usize, price: &str, time: &str) -> w::Change {
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
            ValuationTarget::Collection {
                collection_id: "gift".into(),
            },
            "3",
        ),
        (
            ValuationTarget::Model {
                collection_id: "gift".into(),
                model_id: "m".into(),
            },
            "10",
        ),
        (
            ValuationTarget::Backdrop {
                collection_id: "gift".into(),
                backdrop_id: "b".into(),
            },
            "11",
        ),
        (
            ValuationTarget::ModelBackdrop {
                collection_id: "gift".into(),
                model_id: "m".into(),
                backdrop_id: "b".into(),
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
            request(ValuationTarget::Collection {
                collection_id: "gift".into()
            })
            .period
            .unwrap()
            .from
        );
        assert!(!report.stale);
    }
    let mut mixed = request(ValuationTarget::Collection {
        collection_id: "gift".into(),
    });
    mixed
        .strategies
        .push(StrategyId::new("gift_floor").unwrap());
    let report = handle.query(mixed).await.unwrap();
    assert!(
        matches!(&report.results[3],StrategyOutcome::Error{error,..} if error.code==ValuationErrorCode::UnsupportedTarget)
    );
    assert!(matches!(&report.results[0], StrategyOutcome::Value(_)));
    // Metadata membership errors are not treated as empty evidence.
    let invalid = request(ValuationTarget::Model {
        collection_id: "gift".into(),
        model_id: "unknown".into(),
    });
    assert!(handle.query(invalid).await.is_err());
    assert!(
        handle
            .query(request(ValuationTarget::Model {
                collection_id: "gift".into(),
                model_id: "b".into()
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
        .query(request(ValuationTarget::Collection {
            collection_id: "gift".into(),
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
            .query(request(ValuationTarget::Collection {
                collection_id: "gift".into()
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
    let mut q = request(ValuationTarget::Collection {
        collection_id: "gift".into(),
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
        let mut query = request(ValuationTarget::Criteria {
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
