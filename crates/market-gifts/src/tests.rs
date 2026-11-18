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
    for (min, max) in [(Some(0), None), (None, Some(0)), (Some(10), Some(1))] {
        let mut filters = GiftFilters::new("collection".into(), "GRAM".into());
        filters.number = Some(GiftNumberRange { min, max });
        assert!(filters.validate().is_err());
    }
    for (min, max) in [
        (None, None),
        (Some(1), None),
        (None, Some(10)),
        (Some(10), Some(10)),
    ] {
        assert!(GiftNumberRange { min, max }.validate().is_ok());
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
