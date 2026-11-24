use super::*;
use crate::{gift_floor::tests::book, integration_tests::entity, test_support::MarketDataService};
use market_hub_protocol::wire as w;
use std::sync::Arc;

fn bid(
    id: &str,
    price: &str,
    model: Option<&str>,
    backdrop: Option<&str>,
    pattern: Option<&str>,
    number: Option<i64>,
    expiry: Option<i64>,
    quantity: &str,
) -> w::Change {
    let mut attributes = std::collections::BTreeMap::new();
    for (key, value) in [
        ("tg.model", model),
        ("tg.backdrop", backdrop),
        ("tg.pattern", pattern),
    ] {
        if let Some(value) = value {
            attributes.insert(
                key.into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId(value.into())),
                    })),
                },
            );
        }
    }
    if let Some(number) = number {
        attributes.insert(
            "tg.number".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::Integer(number)),
                })),
            },
        );
    }
    entity(w::entity::Entity::Order(w::Order {
        id: id.into(),
        book: Some(w::BookKey {
            venue_id: "venue".into(),
            item_type_id: "gift".into(),
            currency_id: "GRAM".into(),
        }),
        mode: w::OrderMode::Bid as i32,
        price: price.into(),
        quantity: quantity.into(),
        attributes,
        expires_at_ms: expiry,
        ..Default::default()
    }))
}
#[tokio::test]
async fn quotes_match_all_gift_conditions_and_keep_trait_references_conservative() {
    let (_dir, mut writer) = book();
    writer
        .apply(w::Batch {
            cursor: "bids".into(),
            changes: vec![
                bid("collection", "5", None, None, None, None, None, "10"),
                bid("model", "8", Some("m"), None, None, None, None, "4"),
                bid("backdrop", "7", None, Some("b"), None, None, None, "3"),
                bid("pair", "12", Some("m"), Some("b"), None, None, None, "2"),
                bid("symbol", "6", None, None, Some("symbol"), None, None, "1"),
                bid(
                    "number",
                    "20",
                    Some("m"),
                    Some("b"),
                    Some("symbol"),
                    Some(123),
                    None,
                    "1",
                ),
                bid(
                    "wrong-number",
                    "900",
                    None,
                    None,
                    None,
                    Some(124),
                    None,
                    "1",
                ),
                bid(
                    "wrong-model",
                    "800",
                    Some("otherm"),
                    None,
                    None,
                    None,
                    None,
                    "1",
                ),
                bid("expired", "700", None, None, None, None, Some(1), "1"),
                bid("empty", "600", None, None, None, None, None, "0"),
            ],
        })
        .unwrap();
    let data = MarketDataService::start(writer.reader(), 4, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let query = ValuationQuery {
        target: ValuationTarget::GiftAttributes(GiftAttributes {
            collection_id: "gift".into(),
            traits: GiftTraits {
                model: Some("m".into()),
                backdrop: Some("b".into()),
                pattern: Some("symbol".into()),
            },
            number: Some(GiftNumber::new(123).unwrap()),
        }),
        currency_id: "GRAM".into(),
        strategies: vec![StrategyId::new(BestBid::ID).unwrap()],
        period: None,
    };
    let report = handle.query(query.clone()).await.unwrap();
    let StrategyOutcome::Value(StrategyValuation::BestBid(result)) = &report.results[0] else {
        panic!("{report:?}")
    };
    assert_eq!(result.best.price.value().unwrap().as_str(), "20");
    assert_eq!(result.best.order.as_ref().unwrap().quantity, 1);
    let refs = &result.references;
    assert_eq!(refs.collection.price.value().unwrap().as_str(), "5");
    assert_eq!(
        refs.model.as_ref().unwrap().price.value().unwrap().as_str(),
        "8"
    );
    assert_eq!(
        refs.backdrop
            .as_ref()
            .unwrap()
            .price
            .value()
            .unwrap()
            .as_str(),
        "7"
    );
    assert_eq!(
        refs.model_backdrop
            .as_ref()
            .unwrap()
            .price
            .value()
            .unwrap()
            .as_str(),
        "12"
    );
    assert_eq!(
        refs.pattern
            .as_ref()
            .unwrap()
            .price
            .value()
            .unwrap()
            .as_str(),
        "6"
    );
    assert!(
        Arc::ptr_eq(&report, &handle.query(query.clone()).await.unwrap()),
        "unchanged queries reuse the completed report"
    );
    let mut missing = query.clone();
    let ValuationTarget::GiftAttributes(gift) = &mut missing.target else {
        unreachable!()
    };
    gift.number = None;
    gift.traits.pattern = None;
    let report = handle.query(missing).await.unwrap();
    let StrategyOutcome::Value(StrategyValuation::BestBid(result)) = &report.results[0] else {
        panic!()
    };
    assert_eq!(
        result.best.price.value().unwrap().as_str(),
        "12",
        "unknown traits/number cannot satisfy restricted bids"
    );
    let report = handle
        .query(ValuationQuery {
            target: ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: GiftCriteria {
                    models: vec!["m".into(), "otherm".into()],
                    ..Default::default()
                },
            },
            ..query
        })
        .await
        .unwrap();
    let StrategyOutcome::Value(StrategyValuation::BestBid(result)) = &report.results[0] else {
        panic!()
    };
    assert_eq!(
        result.best.price.value().unwrap().as_str(),
        "5",
        "a selection requires complete coverage, not intersection"
    );
    task.shutdown().await.unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
#[test]
fn criteria_coverage_includes_unrestricted_dimensions_and_number_bounds() {
    let broad = GiftCriteria::default();
    let narrow = GiftCriteria {
        models: vec!["m".into()],
        number: Some(GiftNumberRange {
            min: Some(GiftNumber::new(10).unwrap()),
            max: Some(GiftNumber::new(20).unwrap()),
        }),
        ..Default::default()
    };
    assert!(broad.covers(&narrow));
    assert!(!narrow.covers(&broad));
    assert!(narrow.covers(&narrow));
    assert!(!narrow.accepts(&GiftTraits::default(), None));
    let only_model = GiftCriteria {
        models: vec!["m".into()],
        ..Default::default()
    };
    assert!(!narrow.covers(&only_model));
}

#[tokio::test]
async fn bid_dependencies_and_expiry_invalidate_at_the_shared_refresh_boundary() {
    use crate::{floors::BookCache, market::ValuationRead, source::ValuationSource};
    use tokio_util::sync::CancellationToken;
    let (_dir, mut writer) = book();
    writer
        .apply(w::Batch {
            cursor: "bid-base".into(),
            changes: vec![bid("base", "5", None, None, None, None, None, "1")],
        })
        .unwrap();
    let reads =
        market_replica::ReadRuntime::new(writer.reader(), 2, CancellationToken::new()).unwrap();
    let source = ValuationSource::new(reads.handle());
    let cache = Arc::new(BookCache::default());
    let request = ValuationQuery {
        target: ValuationTarget::Selection {
            collection_id: "gift".into(),
            criteria: Default::default(),
        },
        currency_id: "GRAM".into(),
        strategies: vec![StrategyId::new("best_bid").unwrap()],
        period: None,
    }
    .prepare()
    .unwrap();
    let read = |previous: Option<Arc<ValuationRead>>| {
        let request = request.clone();
        let cache = cache.clone();
        source.prepare(move |snapshot| {
            ValuationRead::read(&snapshot, &request, &cache, previous.as_ref())
        })
    };
    let first = read(None).await.unwrap().unwrap();
    let expiry = jiff::Timestamp::now().as_millisecond() + 1500;
    writer
        .apply(w::Batch {
            cursor: "bid-change".into(),
            changes: vec![bid(
                "higher",
                "9",
                None,
                None,
                None,
                None,
                Some(expiry),
                "1",
            )],
        })
        .unwrap();
    assert!(
        Arc::ptr_eq(&first, &read(Some(first.clone())).await.unwrap().unwrap()),
        "busy books keep their cached quotes between scheduled refreshes"
    );
    crate::test_support::refresh_interval().await;
    let changed = read(Some(first.clone())).await.unwrap().unwrap();
    let StrategyOutcome::Value(StrategyValuation::BestBid(value)) = &changed.report.results[0]
    else {
        panic!()
    };
    assert_eq!(value.best.price.value().unwrap().as_str(), "9");
    tokio::time::sleep(std::time::Duration::from_millis(1550)).await;
    crate::test_support::refresh_interval().await;
    let expired = read(Some(changed)).await.unwrap().unwrap();
    let StrategyOutcome::Value(StrategyValuation::BestBid(value)) = &expired.report.results[0]
    else {
        panic!()
    };
    assert_eq!(
        value.best.price.value().unwrap().as_str(),
        "5",
        "unchanged books still invalidate expired quotes"
    );
    source
        .prepare(move |snapshot| {
            let currency = "GRAM".into();
            let context = ValuationContext::new(&snapshot, &currency, &cache);
            let a = context.bid_orders(&"gift".into()).unwrap();
            let b = context.bid_orders(&"gift".into()).unwrap();
            assert!(
                Arc::ptr_eq(&a, &b),
                "trait and batch reads share one bid-book allocation"
            );
        })
        .await
        .unwrap();
    reads.shutdown().await;
}

#[test]
fn concrete_bids_require_identity_and_unknown_facts_do_not_match_restrictions() {
    let gift = CollectibleGift {
        id: "gift-1".into(),
        number: Some(GiftNumber::new(123).unwrap()),
        collection_id: "gift".into(),
        traits: GiftTraits {
            model: Some("m".into()),
            backdrop: Some("b".into()),
            pattern: None,
        },
    };
    let bid = GiftTarget::Gift { gift: gift.clone() };
    let mut wanted = gift.clone();
    wanted.id = "gift-2".into();
    assert!(!matches_target(
        &bid,
        &ResolvedTarget::Gift {
            gift: wanted,
            slug: None
        }
    ));
    assert!(matches_target(
        &bid,
        &ResolvedTarget::Gift {
            gift: gift.clone(),
            slug: None
        }
    ));
    assert!(!matches_target(
        &bid,
        &ResolvedTarget::Selection {
            collection_id: "gift".into(),
            criteria: Default::default()
        }
    ));
    assert!(!matches_target(
        &bid,
        &ResolvedTarget::GiftAttributes(GiftAttributes {
            collection_id: "gift".into(),
            traits: gift.traits.clone(),
            number: None
        })
    ));
    let unknown = ResolvedTarget::GiftAttributes(GiftAttributes {
        collection_id: "gift".into(),
        traits: Default::default(),
        number: None,
    });
    assert!(!matches_target(
        &GiftTarget::Criteria {
            criteria: GiftCriteria {
                patterns: vec!["symbol".into()],
                ..Default::default()
            }
        },
        &unknown
    ));
    assert!(!matches_target(
        &GiftTarget::Unsupported {
            reason: UnsupportedGiftReason::UnsupportedConstraints
        },
        &unknown
    ));
    let absent = BidQuote::from_order(None);
    assert!(absent.price.value().is_none());
    assert!(absent.order.is_none());
}
