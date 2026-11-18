use market_gifts::*;
#[test]
fn gift_valuation_lookup_accepts_slugs_or_catalog_identity_without_client_supplied_traits() {
    use crate::GiftValuationRequest;
    let request: GiftValuationRequest = serde_json::from_value(serde_json::json!({
        "gift": { "collection_id": "collection", "number": 123 }, "currency_id": "GRAM"
    }))
    .unwrap();
    assert_eq!(
        request.gift,
        GiftLookup::Catalog {
            collection_id: "collection".into(),
            number: GiftNumber::new(123).unwrap()
        }
    );
    let slug: GiftValuationRequest = serde_json::from_value(serde_json::json!({
        "gift": "https://t.me/nft/Gift-00123", "currency_id": "GRAM"
    }))
    .unwrap();
    assert_eq!(
        slug.gift,
        GiftLookup::Slug(GiftSlug::new("Gift-123").unwrap())
    );
    assert_eq!(serde_json::to_value(slug).unwrap()["gift"], "Gift-123");
    for gift in [
        serde_json::json!({"collection_id":"collection","number":0}),
        serde_json::json!({"collection_id":"","number":123}),
        serde_json::json!({"collection_id":"collection","number":1.5}),
        serde_json::json!({"collection_id":"collection"}),
        serde_json::json!({"collection_id":"collection","number":123,"traits":{"model":"forged"}}),
        serde_json::json!({"collection_id":"collection","number":123,"slug":"Gift-999"}),
    ] {
        assert!(
            serde_json::from_value::<GiftValuationRequest>(serde_json::json!({
                "gift": gift, "currency_id": "GRAM"
            }))
            .is_err()
        );
    }
}

#[test]
fn valuation_validates_positive_positions_names_and_total_overflow() {
    use crate::{ModelPosition, ValuationRequest};
    use market_gifts::ModelLookup;
    let mut request = ValuationRequest {
        strategy: crate::StrategyId::model_floor(),
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
    use crate::{ModelPosition, ValuationRequest};
    use market_gifts::ModelLookup;
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
            strategy: crate::StrategyId::model_floor(),
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
                strategy: crate::StrategyId::model_floor(),
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
    use crate::{StrategyId, ValuationRequest};
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
