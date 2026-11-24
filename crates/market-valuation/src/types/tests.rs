#[test]
fn gift_queries_accept_validated_lookups_and_reject_forged_facts() {
    use crate::{ValuationQuery, ValuationTarget};
    for input in [
        serde_json::json!("https://t.me/nft/Gift-00123"),
        serde_json::json!({"collection_id":"collection","number":123}),
    ] {
        let query:ValuationQuery=serde_json::from_value(serde_json::json!({"target":{"kind":"gift","gift":input},"currency_id":"GRAM","strategies":["gift_floor"]})).unwrap();
        assert!(matches!(query.target, ValuationTarget::Gift { .. }));
    }
    for gift in [
        serde_json::json!("Gift-0"),
        serde_json::json!({"collection_id":"collection","number":123,"traits":{"model":"forged"}}),
    ] {
        assert!(serde_json::from_value::<ValuationQuery>(serde_json::json!({"target":{"kind":"gift","gift":gift},"currency_id":"GRAM","strategies":["gift_floor"]})).is_err());
    }
}

#[test]
fn valuation_strategy_ids_are_stable_validated_and_selection_is_required() {
    use crate::StrategyId;
    for id in ["gift_floor", "ask_floor", "catalog_floor", "strategy_v2"] {
        let parsed = StrategyId::new(id).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), id);
    }
    for id in ["", "ModelFloor", "1floor", "a-b", "a/b", &"x".repeat(65)] {
        assert!(StrategyId::new(id).is_err());
    }
}
