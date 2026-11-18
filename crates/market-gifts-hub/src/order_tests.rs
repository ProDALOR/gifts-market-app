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
    let GiftTarget::Gift { gift } = projected.target else {
        panic!("expected concrete gift")
    };
    assert_eq!(gift.id.as_str(), "9007199254740993");
    assert_eq!(gift.number, Some(42));
    assert_eq!(gift.traits.model.as_deref(), Some("model"));
    let mut conditional = fixture();
    conditional.item_id = None;
    conditional.mode = w::OrderMode::Bid as i32;
    assert!(matches!(
        order(conditional).unwrap().target,
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
        order(unknown).unwrap().target,
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
        order(multiple.clone()).unwrap().target,
        GiftTarget::Unsupported { .. }
    ));
    multiple.item_id = None;
    let GiftTarget::Criteria { criteria } = order(multiple).unwrap().target else {
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
        assert_eq!(concrete_model(&raw), Some("model"));
        let projected = order(raw.clone()).unwrap();
        let quote = floor_quote(raw, 0).unwrap().unwrap();
        assert_eq!(quote.price, projected.price);
        assert_eq!(quote.venue_id, projected.book.venue_id);
        let GiftTarget::Gift { gift } = projected.target else {
            panic!("expected concrete gift")
        };
        assert_eq!(Some(quote.model_id), gift.traits.model);
    }
}

#[test]
fn floors_exclude_unsupported_expired_and_nonconcrete_listings() {
    let mut unknown = fixture();
    unknown.attributes.insert(
        "future.condition".into(),
        eq(w::attribute_value::Value::Boolean(true)),
    );
    assert!(floor_quote(unknown, 0).unwrap().is_none());
    let mut multiple = fixture();
    multiple
        .attributes
        .insert(MODEL.into(), models(&["model", "other"]));
    assert_eq!(concrete_model(&multiple), None);
    assert!(floor_quote(multiple, 0).unwrap().is_none());
    let mut nonconcrete = fixture();
    nonconcrete
        .attributes
        .insert(BACKDROP.into(), models(&["one", "two"]));
    assert!(floor_quote(nonconcrete, 0).unwrap().is_none());
    let mut expired = fixture();
    expired.expires_at_ms = Some(10);
    assert!(floor_quote(expired, 10).unwrap().is_none());
    let mut bid = fixture();
    bid.mode = w::OrderMode::Bid as i32;
    assert!(floor_quote(bid, 0).unwrap().is_none());
    for quantity in ["0", "-1"] {
        let mut exhausted = fixture();
        exhausted.quantity = quantity.into();
        assert!(order(exhausted.clone()).is_err());
        assert!(floor_quote(exhausted, 0).unwrap().is_none());
    }
    for quantity in ["0.5", "1.5", "4294967296", "invalid"] {
        let mut invalid = fixture();
        invalid.quantity = quantity.into();
        assert!(order(invalid.clone()).is_err());
        assert!(floor_quote(invalid, 0).is_err());
    }
}

#[test]
fn collection_and_model_floors_share_eligibility_but_differ_on_model_scope() {
    let mut broad = fixture();
    broad.attributes.remove(MODEL);
    assert!(floor_quote(broad.clone(), 0).unwrap().is_none());
    assert!(collection_floor_quote(broad, 0).unwrap().is_some());
    for (price, quantity, expiry) in [("0", "1", None), ("1", "0", None), ("1", "1", Some(10))] {
        let mut raw = fixture();
        raw.price = price.into();
        raw.quantity = quantity.into();
        raw.expires_at_ms = expiry;
        assert!(floor_quote(raw.clone(), 10).unwrap().is_none());
        assert!(collection_floor_quote(raw, 10).unwrap().is_none());
    }
    let mut unknown = fixture();
    unknown.attributes.insert(
        "future.condition".into(),
        eq(w::attribute_value::Value::Boolean(true)),
    );
    assert!(collection_floor_quote(unknown, 0).unwrap().is_none());
}
