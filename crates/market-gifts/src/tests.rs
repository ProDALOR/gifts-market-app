use crate::*;

#[test]
fn standalone_gifts_keep_collection_identity_and_omit_unknown_description() {
    let value = serde_json::json!({"id":"gift","collection_id":"collection","traits":{}});
    let gift: CollectibleGift = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(gift.number, None);
    assert_eq!(gift.traits, GiftTraits::default());
    assert_eq!(serde_json::to_value(gift).unwrap(), value);
    let criteria = GiftCriteria::default();
    assert_eq!(
        serde_json::to_value(&criteria).unwrap(),
        serde_json::json!({"models":[],"backdrops":[],"patterns":[]})
    );
    assert_eq!(
        serde_json::from_value::<GiftCriteria>(serde_json::to_value(&criteria).unwrap()).unwrap(),
        criteria
    );
}

#[test]
fn identifiers_and_exact_amounts_are_validated_at_the_json_boundary() {
    for invalid in ["", " bad", "bad\n", &"x".repeat(257)] {
        assert!(CollectionId::new(invalid).is_err());
        assert!(serde_json::from_str::<ModelId>(&serde_json::to_string(invalid).unwrap()).is_err());
    }
    let amount = Amount::new("10000000000000000.000000001").unwrap();
    assert_eq!(
        serde_json::to_string(&amount).unwrap(),
        "\"10000000000000000.000000001\""
    );
    assert_eq!(Amount::new("1.2500").unwrap().as_str(), "1.25");
    assert!(Amount::new("-1").is_err());
    assert!(serde_json::from_str::<Amount>("1.25").is_err());
    let document = TelegramDocumentId::new(5_375_055_028_669_147_853).unwrap();
    assert_eq!(
        serde_json::to_string(&document).unwrap(),
        "\"5375055028669147853\""
    );
    assert!(TelegramDocumentId::new(0).is_err());
}

#[test]
fn gift_queries_accept_named_traits_and_reject_the_generic_contract() {
    let query: GiftOrderQuery = serde_json::from_value(serde_json::json!({"filters": {
        "collection_id":"collection", "currency_id":"GRAM", "models":["model"],
        "number":{"min":1,"max":100}
    }}))
    .unwrap();
    assert_eq!(query.filters.models[0].as_str(), "model");
    for value in [
        serde_json::json!({"scope":{"attributes":{}}}),
        serde_json::json!({"filters":{}}),
        serde_json::json!({"filters":{"collection_id":"collection"}}),
        serde_json::json!({"filters":{"collection_id":"collection","currency_id":"GRAM"},"side":"executed"}),
        serde_json::json!({"filters":{"attributes":{}}}),
        serde_json::json!({"filters":{"models":[""]}}),
        serde_json::json!({"filters":{"number":{"min":-1}}}),
    ] {
        assert!(serde_json::from_value::<GiftOrderQuery>(value).is_err());
    }
}

#[test]
fn backdrop_colors_are_validated_and_normalized() {
    let color: RgbColor = serde_json::from_str("\"#AbC123\"").unwrap();
    assert_eq!(serde_json::to_string(&color).unwrap(), "\"#abc123\"");
    for invalid in ["red", "#fff", "#gg0000", "#11223344", "#12345 "] {
        assert!(RgbColor::try_from(invalid.to_owned()).is_err());
    }
}

#[test]
fn domain_ranges_reject_reversed_bounds_and_zero_gift_numbers() {
    for value in [serde_json::json!({"min":0}), serde_json::json!({"max":0})] {
        assert!(serde_json::from_value::<GiftNumberRange>(value).is_err());
    }
    let mut filters = GiftFilters::new("collection".into(), "GRAM".into());
    filters.number = Some(GiftNumberRange {
        min: Some(GiftNumber::new(10).unwrap()),
        max: Some(GiftNumber::new(1).unwrap()),
    });
    assert!(filters.validate().is_err());
    for (min, max) in [
        (None, None),
        (Some(1), None),
        (None, Some(10)),
        (Some(10), Some(10)),
    ] {
        assert!(
            GiftNumberRange {
                min: min.map(|n| GiftNumber::new(n).unwrap()),
                max: max.map(|n| GiftNumber::new(n).unwrap())
            }
            .validate()
            .is_ok()
        );
    }
    let mut filters = GiftFilters::new("collection".into(), "GRAM".into());
    filters.price = Some(PriceRange {
        min: Some("10".into()),
        max: Some("1".into()),
    });
    assert!(filters.validate().is_err());
    assert!(
        PriceRange {
            min: Some("1".into()),
            max: Some("1".into())
        }
        .validate()
        .is_ok()
    );
    let earlier = "2026-01-01T00:00:00Z".parse().unwrap();
    let later = "2026-01-02T00:00:00Z".parse().unwrap();
    assert!(
        Period {
            from: Some(later),
            until: Some(earlier)
        }
        .validate()
        .is_err()
    );
    assert!(
        Period {
            from: Some(earlier),
            until: Some(later)
        }
        .validate()
        .is_ok()
    );
    assert!(
        Period {
            from: Some(later),
            until: None
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn gift_slugs_normalize_public_urls_and_reject_untrusted_or_malformed_inputs() {
    for (input, expected) in [
        ("PlushPepe-123", "PlushPepe-123"),
        (" https://t.me/nft/PlushPepe-00123 ", "PlushPepe-123"),
        ("t.me/nft/PlushPepe-123?source=test", "PlushPepe-123"),
        ("https://telegram.me/nft/PlushPepe-123", "PlushPepe-123"),
    ] {
        let slug = crate::GiftSlug::new(input).unwrap();
        assert_eq!(slug.to_string(), expected);
        assert_eq!(slug.number().get(), 123);
        assert!(slug.collection() == &CollectionSlug::new("PlushPepe").unwrap());
        assert_eq!(
            serde_json::from_str::<crate::GiftSlug>(&serde_json::to_string(input).unwrap())
                .unwrap(),
            slug
        );
    }
    for input in [
        "",
        "Gift",
        "Gift-0",
        "Gift--1",
        "Gift-4294967296",
        "Gift-1x",
        "https://evil.example/nft/Gift-1",
        "https://t.me.evil.example/nft/Gift-1",
        "https://user@t.me/nft/Gift-1",
        "https://t.me/nft/Gift-1/other",
        "tg://resolve?domain=Gift-1",
    ] {
        assert!(crate::GiftSlug::new(input).is_err(), "{input}");
    }
}

#[test]
fn gift_numbers_are_positive_in_gifts_ranges_and_json_schemas() {
    for n in [1, u32::MAX] {
        let number = GiftNumber::new(n).unwrap();
        assert_eq!(number.get(), n);
        assert_eq!(serde_json::to_value(number).unwrap(), n);
        assert_eq!(
            serde_json::from_value::<GiftNumber>(n.into()).unwrap(),
            number
        );
    }
    assert!(GiftNumber::new(0).is_err());
    for value in [
        serde_json::json!(0),
        serde_json::json!(-1),
        serde_json::json!(4294967296_u64),
        serde_json::json!(1.5),
        serde_json::json!("1"),
    ] {
        let gift = serde_json::json!({"id":"gift","collection_id":"collection","number":value,"traits":{"model":null,"backdrop":null,"pattern":null}});
        assert!(serde_json::from_value::<CollectibleGift>(gift).is_err());
    }
    #[cfg(feature = "schema")]
    {
        let schema = serde_json::to_value(<GiftNumber as utoipa::PartialSchema>::schema()).unwrap();
        assert_eq!(schema["type"], "integer");
        assert_eq!(schema["minimum"], 1);
        assert_eq!(schema["maximum"], u32::MAX);
    }
}

#[test]
fn slug_equality_ordering_and_hashing_use_normalized_keys_and_preserve_spelling() {
    use std::collections::{BTreeSet, HashSet};
    let a = GiftSlug::new("PlushPepe-0001").unwrap();
    let b = GiftSlug::new("https://t.me/nft/plushpepe-1").unwrap();
    assert_eq!(a, b);
    assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);
    assert_eq!(a.lookup_key(), "plushpepe-1");
    assert_eq!(a.collection().lookup_key(), "plushpepe");
    assert_eq!(serde_json::to_value(&a).unwrap(), "PlushPepe-1");
    assert_eq!(serde_json::to_value(a.collection()).unwrap(), "PlushPepe");
    assert_eq!(HashSet::from([a.clone(), b.clone()]).len(), 1);
    assert_eq!(BTreeSet::from([a, b]).len(), 1);
    for value in [
        "",
        "Plush Pepe",
        "Gift-1",
        "Gift/Other",
        " Gift",
        "💎",
        &"x".repeat(129),
    ] {
        assert!(CollectionSlug::new(value).is_err());
    }
}

fn order_data() -> GiftOrderData {
    GiftOrderData {
        id: "listing".into(),
        book: GiftBook {
            venue_id: "venue".into(),
            collection_id: "collection".into(),
            currency_id: "GRAM".into(),
        },
        kind: GiftOrderKind::Ask,
        target: GiftTarget::Gift {
            gift: CollectibleGift {
                id: "gift".into(),
                collection_id: "collection".into(),
                number: Some(GiftNumber::new(1).unwrap()),
                traits: GiftTraits::default(),
            },
        },
        price: "10".into(),
        quantity: 1,
        observed_at: "2026-01-01T00:00:00Z".parse().unwrap(),
        expires_at: None,
        executed_at: None,
        market_url: None,
    }
}
#[test]
fn checked_order_construction_enforces_identity_quantity_and_partial_gift_invariants() {
    let data = order_data();
    let order = GiftOrder::try_from(data.clone()).unwrap();
    assert_eq!(order.quantity, 1);
    let mut invalid = data.clone();
    invalid.book.collection_id = "other".into();
    assert!(GiftOrder::try_from(invalid).is_err());
    for quantity in [0, 2, u32::MAX] {
        let mut invalid = data.clone();
        invalid.quantity = quantity;
        assert!(GiftOrder::try_from(invalid).is_err());
    }
    let mut partial = data.clone();
    if let GiftTarget::Gift { gift } = &mut partial.target {
        gift.number = None;
    }
    assert!(GiftOrder::try_from(partial).is_ok());
    let mut criteria = data;
    criteria.target = GiftTarget::Criteria {
        criteria: GiftCriteria::default(),
    };
    criteria.quantity = 2;
    assert!(GiftOrder::try_from(criteria.clone()).is_ok());
    criteria.quantity = 0;
    assert!(GiftOrder::try_from(criteria.clone()).is_err());
    criteria.quantity = 2;
    let GiftTarget::Criteria { criteria: target } = &mut criteria.target else {
        unreachable!()
    };
    target.number = Some(GiftNumberRange {
        min: Some(GiftNumber::new(10).unwrap()),
        max: Some(GiftNumber::new(1).unwrap()),
    });
    assert!(GiftOrder::try_from(criteria).is_err());
}
#[test]
fn checked_orders_require_execution_time_exactly_for_executions() {
    for kind in [
        GiftOrderKind::Ask,
        GiftOrderKind::Bid,
        GiftOrderKind::Executed,
    ] {
        for executed_at in [None, Some("2025-12-01T00:00:00Z".parse().unwrap())] {
            let mut data = order_data();
            data.kind = kind;
            data.executed_at = executed_at;
            assert_eq!(
                GiftOrder::try_from(data).is_ok(),
                (kind == GiftOrderKind::Executed) == executed_at.is_some()
            );
        }
    }
    // Editing an extracted draft never mutates the valid value that consumers already hold.
    let order = GiftOrder::try_from(order_data()).unwrap();
    let mut draft = order.clone().into_data();
    draft.quantity = 2;
    assert!(GiftOrder::try_from(draft).is_err());
    assert_eq!(order.quantity, 1);
}

#[test]
fn one_query_resolves_source_defaults_and_rejects_ambiguous_navigation_and_views() {
    for (source, expected) in [
        (OrderSource::Asks, GiftSort::PriceAsc),
        (OrderSource::Bids, GiftSort::PriceDesc),
        (OrderSource::History, GiftSort::TimeDesc),
    ] {
        let mut query = GiftOrderQuery::new(GiftFilters::new("gift".into(), "GRAM".into()));
        query.source = source;
        assert_eq!(query.sort(), expected);
        query.sort = Some(GiftSort::NumberAsc);
        assert_eq!(query.sort(), GiftSort::NumberAsc);
    }
    for navigation in [
        serde_json::json!({"type":"head","key":"aa"}),
        serde_json::json!({"type":"after"}),
        serde_json::json!({"type":"after","key":"ABC=EF"}),
        serde_json::json!({"type":"around","key":"a"}),
    ] {
        assert!(serde_json::from_value::<Navigation>(navigation).is_err());
    }
    assert!(
        serde_json::from_value::<ReadView>(
            serde_json::json!({"kind":"latest","id":"00000000-0000-0000-0000-000000000001"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<ReadView>(serde_json::json!({"kind":"snapshot","id":"bad"}))
            .is_err()
    );
    let key: OrderKey = serde_json::from_str("\"0123ab\"").unwrap();
    assert_eq!(key.as_str(), "0123ab");
    for raw in ["", "g-g", "ab ", &"ff".repeat(4097)] {
        assert!(OrderKey::try_from(raw.to_owned()).is_err());
    }
}
