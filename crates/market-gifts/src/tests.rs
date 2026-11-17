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
