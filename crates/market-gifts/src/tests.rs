use crate::*;

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
    let query: GiftOrdersQuery = serde_json::from_value(serde_json::json!({"filters": {
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
        assert!(serde_json::from_value::<GiftOrdersQuery>(value).is_err());
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
fn valuation_validates_positive_positions_names_and_total_overflow() {
    use crate::valuation::{ModelLookup, ModelPosition, ValuationRequest};
    let mut request = ValuationRequest {
        strategy: crate::valuation::StrategyId::model_floor(),
        positions: vec![ModelPosition {
            model: ModelLookup::Name {
                collection: "Gift".into(),
                model: "Model".into(),
            },
            quantity: 0,
        }],
        currency_id: "GRAM".into(),
    };
    assert!(request.validate().is_err());
    request.positions[0].quantity = 1;
    assert!(request.validate().is_ok());
    request.positions[0].model = ModelLookup::Name {
        collection: " ".into(),
        model: "Model".into(),
    };
    assert!(request.validate().is_err());
    request.positions[0].model = ModelLookup::Catalog {
        collection: "collection".into(),
        model: "model".into(),
    };
    request.positions[0].quantity = u32::MAX;
    request.positions.push(request.positions[0].clone());
    assert!(request.validate().is_err());
}

#[test]
fn named_lookups_and_portfolios_share_the_same_name_validation() {
    use crate::valuation::{ModelLookup, ModelPosition, ValuationRequest};
    for name in [
        "".to_owned(),
        " \t".to_owned(),
        "x".repeat(257),
        "💎".repeat(65),
    ] {
        assert!(ModelLookup::named(name.clone(), "Model").is_err());
        assert!(ModelLookup::named("Collection", name.clone()).is_err());
        let raw = ModelLookup::Name {
            collection: "Collection".into(),
            model: name,
        };
        assert!(raw.validate().is_err());
        let request = ValuationRequest {
            strategy: crate::valuation::StrategyId::model_floor(),
            positions: vec![ModelPosition {
                model: raw,
                quantity: 1,
            }],
            currency_id: "GRAM".into(),
        };
        assert!(request.validate().is_err());
    }
    for name in ["x".repeat(256), "💎".repeat(64)] {
        let model = ModelLookup::named("Collection", name).unwrap();
        assert!(
            ValuationRequest {
                strategy: crate::valuation::StrategyId::model_floor(),
                positions: vec![ModelPosition { model, quantity: 1 }],
                currency_id: "GRAM".into()
            }
            .validate()
            .is_ok()
        );
    }
}

#[test]
fn valuation_strategy_ids_are_stable_validated_and_selection_is_required() {
    use crate::valuation::{StrategyId, ValuationRequest};
    assert!(
        serde_json::from_value::<ValuationRequest>(
            serde_json::json!({"positions":[],"currency_id":"GRAM"})
        )
        .is_err()
    );
    for id in ["model_floor", "collection_floor", "strategy_v2"] {
        let parsed = StrategyId::new(id).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), id);
    }
    for id in ["", "ModelFloor", "1floor", "a-b", "a/b", &"x".repeat(65)] {
        assert!(StrategyId::new(id).is_err());
    }
}

#[test]
fn multiplier_ratios_validate_baselines_and_round_deterministically() {
    use crate::valuation::*;
    assert_eq!(
        Multiplier::from_floors(&"10".into(), &"3".into())
            .unwrap()
            .as_str(),
        "3.333333333333"
    );
    assert_eq!(
        Multiplier::from_floors(&"6".into(), &"2".into())
            .unwrap()
            .as_str(),
        "3"
    );
    assert!(Multiplier::from_floors(&"2".into(), &"0".into()).is_err());
    for value in ["0", "-1", "invalid"] {
        assert!(Multiplier::try_from(value.to_owned()).is_err());
    }
    for value in [
        serde_json::json!({"collection_id":"gift","model_id":"model","currency_id":"GRAM"}),
        serde_json::json!({"strategy":"model_multiplier","collection_id":"gift","backdrop_id":"backdrop","currency_id":"GRAM"}),
        serde_json::json!({"strategy":"backdrop_multiplier","collection_id":"gift","backdrop_id":"backdrop","currency_id":"GRAM","positions":[]}),
    ] {
        assert!(serde_json::from_value::<ValuationQuery>(value).is_err());
    }
    let request:ValuationQuery=serde_json::from_value(serde_json::json!({"strategy":"model_multiplier","collection_id":"gift","model_id":"model","currency_id":"GRAM"})).unwrap();
    assert!(matches!(request, ValuationQuery::ModelMultiplier(_)));
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

fn order_json() -> serde_json::Value {
    serde_json::json!({
        "id":"listing", "book":{"venue_id":"venue","collection_id":"collection","currency_id":"GRAM"},
        "kind":"ask", "target":{"kind":"gift","gift":{"id":"gift","collection_id":"collection","number":1,"traits":{"model":null,"backdrop":null,"pattern":null}}},
        "price":"10", "quantity":1, "observed_at":"2026-01-01T00:00:00Z",
        "expires_at":null, "executed_at":null, "parent_order_id":null, "market_url":null
    })
}

#[test]
fn shared_order_validation_covers_json_projection_invariants_and_partial_gifts() {
    let json = order_json();
    let order: GiftOrder = serde_json::from_value(json.clone()).unwrap();
    assert!(order.validate().is_ok());
    assert_eq!(serde_json::to_value(&order).unwrap(), json);
    let mut invalid = order.clone();
    invalid.book.collection_id = "other".into();
    assert!(invalid.validate().is_err());
    assert!(serde_json::to_value(&invalid).is_err());
    for quantity in [0, 2, u32::MAX] {
        let mut invalid = order.clone();
        invalid.quantity = quantity;
        assert!(invalid.validate().is_err());
        assert!(serde_json::to_value(&invalid).is_err());
        let mut invalid_json = json.clone();
        invalid_json["quantity"] = quantity.into();
        assert!(serde_json::from_value::<GiftOrder>(invalid_json).is_err());
    }
    let mut mismatch = json.clone();
    mismatch["target"]["gift"]["collection_id"] = "other".into();
    assert!(serde_json::from_value::<GiftOrder>(mismatch).is_err());
    let mut unknown_field = json.clone();
    unknown_field["legacy"] = true.into();
    assert!(serde_json::from_value::<GiftOrder>(unknown_field).is_err());
    let mut partial = json.clone();
    partial["target"]["gift"]["number"] = serde_json::Value::Null;
    assert!(
        serde_json::from_value::<GiftOrder>(partial)
            .unwrap()
            .validate()
            .is_ok()
    );

    let mut criteria = json;
    criteria["target"] =
        serde_json::json!({"kind":"criteria","criteria":{"number":{"min":1,"max":10}}});
    criteria["quantity"] = 2.into();
    assert!(
        serde_json::from_value::<GiftOrder>(criteria.clone())
            .unwrap()
            .validate()
            .is_ok()
    );
    criteria["quantity"] = 0.into();
    assert!(serde_json::from_value::<GiftOrder>(criteria.clone()).is_err());
    criteria["quantity"] = 2.into();
    criteria["target"]["criteria"]["number"] = serde_json::json!({"min":10,"max":1});
    assert!(serde_json::from_value::<GiftOrder>(criteria).is_err());
}
