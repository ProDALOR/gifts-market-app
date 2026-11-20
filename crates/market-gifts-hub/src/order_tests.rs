use super::*;
use market_gifts::*;
use market_hub_protocol::wire as w;
fn eq(value: w::attribute_value::Value) -> w::AttributePredicate {
    w::AttributePredicate {
        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
            value: Some(value),
        })),
    }
}
fn fixture() -> w::Order {
    w::Order {
        id: "order".into(),
        book: Some(w::BookKey {
            venue_id: "venue".into(),
            item_type_id: "collection".into(),
            currency_id: "GRAM".into(),
        }),
        mode: w::OrderMode::Ask as i32,
        item_id: Some("9007199254740993".into()),
        price: "10000000000000000.000000001".into(),
        quantity: "1.00".into(),
        attributes: [
            (
                MODEL.into(),
                eq(w::attribute_value::Value::OptionId("model".into())),
            ),
            (NUMBER.into(), eq(w::attribute_value::Value::Integer(42))),
        ]
        .into(),
        ..Default::default()
    }
}
#[test]
fn projection_preserves_exact_amounts_and_distinct_concrete_identity() {
    let projected = order(fixture()).unwrap();
    assert_eq!(projected.price.as_str(), "10000000000000000.000000001");
    assert_eq!(projected.quantity, 1);
    let GiftTarget::Gift { gift } = projected.into_data().target else {
        panic!("expected concrete gift")
    };
    assert_eq!(gift.id.as_str(), "9007199254740993");
    assert_eq!(gift.number, Some(GiftNumber::new(42).unwrap()));
    assert_eq!(gift.traits.model.as_deref(), Some("model"));
    let mut conditional = fixture();
    conditional.item_id = None;
    conditional.mode = w::OrderMode::Bid as i32;
    assert!(matches!(
        order(conditional).unwrap().into_data().target,
        GiftTarget::Criteria { .. }
    ));
}
#[test]
fn unsupported_or_nonconcrete_conditions_are_never_reported_as_any_or_as_a_collectible() {
    let mut unknown = fixture();
    unknown.attributes.insert(
        "future.condition".into(),
        eq(w::attribute_value::Value::Boolean(true)),
    );
    assert!(matches!(
        order(unknown).unwrap().into_data().target,
        GiftTarget::Unsupported { .. }
    ));
    let mut multiple = fixture();
    multiple.attributes.insert(
        MODEL.into(),
        w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::OneOf(
                w::AttributeValues {
                    values: vec![
                        w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId("a".into())),
                        },
                        w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId("b".into())),
                        },
                    ],
                },
            )),
        },
    );
    assert!(matches!(
        order(multiple.clone()).unwrap().into_data().target,
        GiftTarget::Unsupported { .. }
    ));
    multiple.item_id = None;
    let GiftTarget::Criteria { criteria } = order(multiple).unwrap().into_data().target else {
        panic!("expected selection")
    };
    assert_eq!(criteria.models.len(), 2);
    let mut fractional = fixture();
    fractional.quantity = "1.5".into();
    assert!(order(fractional).is_err());
    let mut negative = fixture();
    negative.price = "-1".into();
    assert!(order(negative).is_err());
}

fn models(ids: &[&str]) -> w::AttributePredicate {
    w::AttributePredicate {
        predicate: Some(w::attribute_predicate::Predicate::OneOf(
            w::AttributeValues {
                values: ids
                    .iter()
                    .map(|id| w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId((*id).into())),
                    })
                    .collect(),
            },
        )),
    }
}

#[test]
fn floors_and_order_projection_agree_on_singleton_and_duplicate_model_predicates() {
    for predicate in [
        eq(w::attribute_value::Value::OptionId("model".into())),
        models(&["model"]),
        models(&["model", "model"]),
    ] {
        let mut raw = fixture();
        raw.attributes.insert(MODEL.into(), predicate);
        let projected = order(raw.clone()).unwrap();
        let quote = ask_quote(raw, 0).unwrap().unwrap();
        assert_eq!(quote.price, projected.price);
        assert_eq!(quote.venue_id, projected.book.venue_id);
        let GiftTarget::Gift { gift } = projected.into_data().target else {
            panic!("expected concrete gift")
        };
        assert_eq!(quote.gift_id.as_ref(), Some(&gift.id));
        assert_eq!(quote.model_id, gift.traits.model);
    }
}

#[test]
fn floors_exclude_unsupported_expired_and_nonconcrete_listings() {
    let mut unknown = fixture();
    unknown.attributes.insert(
        "future.condition".into(),
        eq(w::attribute_value::Value::Boolean(true)),
    );
    assert!(ask_quote(unknown, 0).unwrap().is_none());
    let mut multiple = fixture();
    multiple
        .attributes
        .insert(MODEL.into(), models(&["model", "other"]));
    assert!(ask_quote(multiple, 0).unwrap().is_none());
    let mut nonconcrete = fixture();
    nonconcrete
        .attributes
        .insert(BACKDROP.into(), models(&["one", "two"]));
    assert!(ask_quote(nonconcrete, 0).unwrap().is_none());
    let mut expired = fixture();
    expired.expires_at_ms = Some(10);
    assert!(ask_quote(expired, 10).unwrap().is_none());
    let mut bid = fixture();
    bid.mode = w::OrderMode::Bid as i32;
    assert!(ask_quote(bid, 0).unwrap().is_none());
    for quantity in ["0", "-1"] {
        let mut exhausted = fixture();
        exhausted.quantity = quantity.into();
        assert!(order(exhausted.clone()).is_err());
        assert!(ask_quote(exhausted, 0).unwrap().is_none());
    }
    for quantity in ["0.5", "1.5", "4294967296", "invalid"] {
        let mut invalid = fixture();
        invalid.quantity = quantity.into();
        assert!(order(invalid.clone()).is_err());
        assert!(ask_quote(invalid, 0).is_err());
    }
}

#[test]
fn collection_and_model_floors_share_eligibility_but_differ_on_model_scope() {
    let mut broad = fixture();
    broad.attributes.remove(MODEL);
    assert!(
        ask_quote(broad.clone(), 0)
            .unwrap()
            .unwrap()
            .model_id
            .is_none()
    );
    assert!(ask_quote(broad, 0).unwrap().is_some());
    for (price, quantity, expiry) in [("0", "1", None), ("1", "0", None), ("1", "1", Some(10))] {
        let mut raw = fixture();
        raw.price = price.into();
        raw.quantity = quantity.into();
        raw.expires_at_ms = expiry;
        assert!(ask_quote(raw.clone(), 10).unwrap().is_none());
        assert!(ask_quote(raw, 10).unwrap().is_none());
    }
    let mut unknown = fixture();
    unknown.attributes.insert(
        "future.condition".into(),
        eq(w::attribute_value::Value::Boolean(true)),
    );
    assert!(ask_quote(unknown, 0).unwrap().is_none());
}

#[test]
fn trait_evidence_never_invents_a_model_from_broad_criteria() {
    let mut raw = fixture();
    raw.item_id = None;
    raw.attributes
        .insert(MODEL.into(), models(&["model", "other"]));
    raw.attributes
        .insert(BACKDROP.into(), models(&["backdrop", "backdrop"]));
    let quote = ask_quote(raw, 0).unwrap().unwrap();
    assert!(quote.model_id.is_none());
    assert_eq!(quote.backdrop_id.as_deref(), Some("backdrop"));
}

#[test]
fn concrete_quantities_use_shared_validation_while_criteria_keep_bulk_quantities() {
    for quantity in ["2", "2.00", "4294967295"] {
        let mut raw = fixture();
        raw.quantity = quantity.into();
        assert!(order(raw.clone()).is_err());
        assert!(ask_quote(raw.clone(), 0).is_err());
        raw.item_id = None;
        let projected = order(raw).unwrap();
        assert!(matches!(
            projected.into_data().target,
            GiftTarget::Criteria { .. }
        ));
    }
    for number in [0, -1, i64::MAX] {
        let mut raw = fixture();
        raw.attributes.insert(
            NUMBER.into(),
            eq(w::attribute_value::Value::Integer(number)),
        );
        // Unsupported source constraints remain explicit rather than inventing gift facts.
        assert!(matches!(
            order(raw).unwrap().into_data().target,
            GiftTarget::Unsupported { .. }
        ));
    }
}

#[test]
fn unsupported_targets_have_stable_typed_reasons_without_source_diagnostics() {
    let mut unsupported = fixture();
    unsupported.attributes.insert(
        "private.source.constraint".into(),
        eq(w::attribute_value::Value::Boolean(true)),
    );
    assert!(matches!(
        order(unsupported).unwrap().into_data().target,
        GiftTarget::Unsupported {
            reason: UnsupportedGiftReason::UnsupportedConstraints
        }
    ));
    let mut invalid = fixture();
    invalid.attributes.insert(
        MODEL.into(),
        eq(w::attribute_value::Value::OptionId("".into())),
    );
    assert!(matches!(
        order(invalid).unwrap().into_data().target,
        GiftTarget::Unsupported {
            reason: UnsupportedGiftReason::InvalidAttributes
        }
    ));
    let mut number = fixture();
    number
        .attributes
        .insert(NUMBER.into(), eq(w::attribute_value::Value::Integer(0)));
    assert!(matches!(
        order(number).unwrap().into_data().target,
        GiftTarget::Unsupported {
            reason: UnsupportedGiftReason::InvalidAttributes
        }
    ));
    let mut nonconcrete = fixture();
    nonconcrete
        .attributes
        .insert(MODEL.into(), models(&["a", "b"]));
    assert!(matches!(
        order(nonconcrete).unwrap().into_data().target,
        GiftTarget::Unsupported {
            reason: UnsupportedGiftReason::NonConcreteGift
        }
    ));
    let mut id = fixture();
    id.item_id = Some("".into());
    assert!(matches!(
        order(id).unwrap().into_data().target,
        GiftTarget::Unsupported {
            reason: UnsupportedGiftReason::InvalidGiftId
        }
    ));
}

#[test]
fn execution_projection_enforces_time_and_lineage_and_ignores_unused_history_lineage_contents() {
    for mode in [w::OrderMode::Ask, w::OrderMode::Bid, w::OrderMode::Executed] {
        for time in [None, Some(1)] {
            let mut value = fixture();
            value.mode = mode as i32;
            value.executed_at_ms = time;
            assert_eq!(
                order(value).is_ok(),
                (mode == w::OrderMode::Executed) == time.is_some()
            );
        }
    }
    let mut active = fixture();
    active.parent_order_id = Some("parent".into());
    assert!(order(active).is_err());
    let mut execution = fixture();
    execution.mode = w::OrderMode::Executed as i32;
    execution.executed_at_ms = Some(1);
    execution.parent_order_id = Some("unused lineage is not a gift identifier".repeat(20));
    assert!(order(execution).is_ok());
    for mode in [w::OrderMode::Ask, w::OrderMode::Bid, w::OrderMode::Executed] {
        for quantity in ["0", "0.00", "-1", "1"] {
            let mut value = fixture();
            value.mode = mode as i32;
            value.quantity = quantity.into();
            assert_eq!(
                is_exhausted(&value).unwrap(),
                mode != w::OrderMode::Executed && matches!(quantity, "0" | "0.00")
            );
        }
    }
}

#[test]
fn concrete_gift_facts_share_target_and_quantity_rules_without_presentation_fields() {
    let input = fixture();
    let GiftTarget::Gift { gift: expected } = order(input.clone()).unwrap().into_data().target
    else {
        panic!("concrete fixture");
    };
    let mut facts = input.clone();
    facts.price = "irrelevant".into();
    facts.observed_at_ms = i64::MAX;
    assert_eq!(concrete_gift(facts).unwrap().unwrap(), expected);
    let mut exhausted = input.clone();
    exhausted.quantity = "0".into();
    assert!(concrete_gift(exhausted).unwrap().is_none());
    for quantity in ["-1", "0.5", "bad"] {
        let mut invalid = input.clone();
        invalid.quantity = quantity.into();
        assert!(concrete_gift(invalid).is_err());
    }
    let mut criteria = input;
    criteria.item_id = None;
    assert!(concrete_gift(criteria).unwrap().is_none());
}
