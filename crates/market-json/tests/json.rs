#[path = "../../../tests/support/mod.rs"]
mod support;
use market_json as json;
use market_view as m;
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct NativeFields {
    at: m::MarketTimestamp,
    #[serde(with = "json::u64_string")]
    revision: u64,
    #[serde(with = "m::time::duration::option")]
    age: Option<Duration>,
}

#[test]
fn native_fields_preserve_json_types_exact_revisions_and_timestamp_validation() {
    let value: NativeFields = serde_json::from_value(json!({
        "at": "2026-11-09T18:00:00.123+02:00",
        "revision": u64::MAX.to_string(),
        "age": "1s 234ms"
    }))
    .unwrap();
    assert_eq!(value.revision, u64::MAX);
    assert_eq!(value.age, Some(Duration::from_millis(1234)));
    let encoded = serde_json::to_value(&value).unwrap();
    assert_eq!(encoded["at"], "2026-11-09T16:00:00.123Z");
    assert_eq!(encoded["revision"], u64::MAX.to_string());
    assert_eq!(
        serde_json::from_value::<NativeFields>(encoded.clone()).unwrap(),
        value
    );

    for (field, invalid) in [
        ("at", json!(1794240000123_i64)),
        ("at", json!("2026-11-09T16:00:00.123456Z")),
        ("at", json!("2026-11-09T16:00:00")),
        ("revision", json!(u64::MAX)),
        ("revision", json!("18446744073709551616")),
        ("revision", json!("+1")),
        ("age", json!(-1)),
        ("age", json!(1234)),
        ("age", json!(1.5)),
        ("age", json!("1234")),
    ] {
        let mut bad = encoded.clone();
        bad[field] = invalid;
        assert!(serde_json::from_value::<NativeFields>(bad).is_err());
    }
    let mut finer_age = value;
    finer_age.age = Some(Duration::from_nanos(1_234_567));
    let encoded_age = serde_json::to_value(&finer_age).unwrap();
    assert_eq!(
        humantime::parse_duration(encoded_age["age"].as_str().unwrap()).unwrap(),
        Duration::from_nanos(1_234_567)
    );
    finer_age.age = None;
    assert!(serde_json::to_value(&finer_age).unwrap()["age"].is_null());
    finer_age.age = Some(Duration::from_secs(5 * 60));
    assert_eq!(serde_json::to_value(&finer_age).unwrap()["age"], "5m");
}
fn catalog() -> m::Catalog {
    support::catalog("source")
}
fn record_values() -> Vec<Value> {
    support::records()
        .iter()
        .map(|record| json::record_value(record).unwrap())
        .collect()
}
fn decode(value: Value) -> json::Result<Vec<m::Record>> {
    json::parse_records(&serde_json::to_vec(&vec![value]).unwrap(), &catalog())
}
#[test]
fn compact_records_roundtrip_without_payload_or_scalar_type_wrappers() {
    let catalog = catalog();
    let records =
        json::parse_records(&serde_json::to_vec(&record_values()).unwrap(), &catalog).unwrap();
    assert_eq!(records, support::records());
    assert_eq!(records.len(), 9);
    for record in &records {
        let value = json::record_value(record).unwrap();
        assert!(value.get("data").is_none());
        assert!(value.get("observed_at_ms").is_none());
        assert!(value["observed_at"].as_str().is_some());
        assert_eq!(
            json::parse_records(&serde_json::to_vec(&vec![value]).unwrap(), &catalog).unwrap(),
            vec![record.clone()]
        );
    }
    let values: Vec<_> = support::sources()
        .iter()
        .map(|s| json::source_value(s).unwrap())
        .collect();
    let sources = json::parse_sources(&serde_json::to_vec(&values).unwrap(), &catalog).unwrap();
    assert_eq!(sources, support::sources());
    assert_eq!(sources.len(), 2);
    for source in &sources {
        let value = json::source_value(source).unwrap();
        assert!(value.get("data").is_none());
        assert_eq!(
            json::parse_sources(&serde_json::to_vec(&vec![value]).unwrap(), &catalog).unwrap(),
            vec![source.clone()]
        );
    }
}
#[test]
fn schema_interprets_exact_strings_boolean_false_and_empty_text() {
    let catalog = catalog();
    let kind = "test-skin".parse().unwrap();
    let decoded=json::parse_attributes(br#"{"wear":"0.0000000000000000000000000001","pattern-seed":"661","special-variant":false,"custom-name":"","available-at":"2026-11-09T18:00:00+02:00"}"#,&catalog,&kind).unwrap();
    assert_eq!(
        decoded["wear"],
        m::AttributeValue::decimal("0.0000000000000000000000000001").unwrap()
    );
    assert_eq!(decoded["pattern-seed"], m::AttributeValue::Integer(661));
    assert_eq!(
        decoded["special-variant"],
        m::AttributeValue::Boolean(false)
    );
    assert_eq!(
        decoded["custom-name"],
        m::AttributeValue::Text(String::new())
    );
    assert_eq!(
        json::attributes(&decoded)["available-at"],
        json::Scalar::String("2026-11-09T16:00:00Z".into())
    );
    let large = json::parse_attributes(
        br#"{"serial-number":"9223372036854775807","model":"test-item.model.gold"}"#,
        &catalog,
        &"test-item".parse().unwrap(),
    )
    .unwrap();
    assert_eq!(large["serial-number"], m::AttributeValue::Integer(i64::MAX));
    assert!(matches!(&large["model"], m::AttributeValue::Enum(_)));
}
#[test]
fn compact_values_reject_guessing_unknown_rules_precision_loss_and_duplicates() {
    let catalog = catalog();
    let kind = "test-skin".parse().unwrap();
    for raw in [
        r#"{"wear":0.03}"#,
        r#"{"wear":true}"#,
        r#"{"wear":null}"#,
        r#"{"wear":{"type":"decimal","value":"0.03"}}"#,
        r#"{"wear":"0.00000000000000000000000000001"}"#,
        r#"{"wear":"1.1"}"#,
        r#"{"pattern-seed":"9223372036854775808"}"#,
        r#"{"special-variant":"false"}"#,
        r#"{"unknown":"value"}"#,
        r#"{"available-at":1794240000000}"#,
        r#"{"wear":"0.03","wear":"0.04"}"#,
    ] {
        assert!(
            json::parse_attributes(raw.as_bytes(), &catalog, &kind).is_err(),
            "{raw}"
        );
    }
    assert!(
        json::parse_attributes(
            br#"{"model":"test-item.pattern.stars"}"#,
            &catalog,
            &"test-item".parse().unwrap()
        )
        .is_err()
    );
    assert!(json::parse_attributes(br#"{}"#, &catalog, &"unknown-type".parse().unwrap()).is_err());
}
#[test]
fn flat_records_reject_unknown_or_duplicate_fields_and_invalid_datetime_precision() {
    let item = record_values().remove(0);
    let mut extra = item.clone();
    extra["unexpected"] = true.into();
    assert!(decode(extra).is_err());
    for time in [
        json!(1794240000000_i64),
        json!("2026-11-09T16:00:00"),
        json!("2026-11-09T16:00:00.123456Z"),
        json!("1960-01-01T00:00:00Z"),
    ] {
        let mut bad = item.clone();
        bad["observed_at"] = time;
        assert!(decode(bad).is_err());
    }
    for amount in [json!("1.00000000000000000000000000001"), json!(1.25)] {
        let mut order = record_values().remove(1);
        order["price"]["amount"] = amount;
        assert!(decode(order).is_err());
    }
    let raw = r#"[{"type":"item","feed_id":"source","feed_id":"other","item_type_id":"test-item","observed_at":"2026-11-09T16:00:00Z","item_id":"item","attributes":{}}]"#;
    assert!(json::parse_records(raw.as_bytes(), &catalog()).is_err());
    let raw = r#"[{"type":"item","feed_id":"source","item_type_id":"test-item","observed_at":"2026-11-09T16:00:00Z","item_id":"item","attributes":{"model":"test-item.model.gold","model":"test-item.model.silver"}}]"#;
    assert!(json::parse_records(raw.as_bytes(), &catalog()).is_err());
}
#[test]
fn only_canonical_datetime_names_are_accepted_for_optional_and_source_times() {
    let catalog = catalog();
    let mut order = record_values().remove(1);
    order["expires_at"] = json!("2026-11-10T16:00:00Z");
    let decoded = decode(order.clone()).unwrap();
    assert_eq!(
        json::record_value(&decoded[0]).unwrap()["expires_at"],
        order["expires_at"]
    );
    order.as_object_mut().unwrap().remove("expires_at");
    order["expires_at_ms"] = json!(1794326400000_i64);
    assert!(decode(order).is_err());

    let feed = json!({"type":"feed","feed_id":"source","venue_id":"test-market","state":"ready","last_sync_ms":1794240000000_i64});
    assert!(decode(feed).is_err());
    let sources: Vec<Value> = support::sources()
        .iter()
        .map(|s| json::source_value(s).unwrap())
        .collect();
    for (mut source, field, old) in [
        (sources[0].clone(), "executed_at", "executed_at_ms"),
        (sources[1].clone(), "occurred_at", "occurred_at_ms"),
    ] {
        source.as_object_mut().unwrap().remove(field);
        source[old] = json!(1794239999000_i64);
        assert!(
            json::parse_sources(&serde_json::to_vec(&vec![source]).unwrap(), &catalog).is_err()
        );
    }
}
#[test]
fn bid_predicates_use_catalog_typed_operands_and_keep_strict_bounds() {
    let mut bid = record_values()
        .into_iter()
        .find(|v| v["order_id"] == "skin-bid")
        .unwrap();
    for predicate in [
        json!({"op":"range","min":"0.5","max":"0.1"}),
        json!({"op":"range","min":null}),
        json!({"op":"one_of","values":["0.1","0.10"]}),
        json!({"op":"eq","value":false}),
    ] {
        bid["kind"]["filter"]["wear"] = predicate;
        assert!(decode(bid.clone()).is_err());
    }
    bid["kind"]["filter"]["wear"] = json!({"op":"range","max":"0.035"});
    assert!(decode(bid).is_ok());
    let mut bid = record_values()
        .into_iter()
        .find(|v| v["order_id"] == "bid-1")
        .unwrap();
    bid["kind"]["filter"]["model"]["values"] = json!(["test-item.pattern.stars"]);
    assert!(decode(bid).is_err());
}
#[test]
fn datetime_bounds_and_feed_statuses_are_string_values() {
    let time = "2026-11-09T18:00:00.123+02:00"
        .parse::<m::MarketTimestamp>()
        .unwrap();
    let entry = m::CatalogEntry::Attribute(m::AttributeDefinition {
        id: "date".parse().unwrap(),
        name: "Date".into(),
        value_type: m::AttributeType::Timestamp,
        minimum: Some(m::AttributeValue::Timestamp(time)),
        maximum: None,
        max_length: None,
    });
    assert_eq!(
        json::catalog_value(&entry).unwrap()["minimum"],
        "2026-11-09T16:00:00.123Z"
    );
    let feed = m::Record::Feed(m::FeedStatus {
        feed_id: "source".parse().unwrap(),
        venue_id: "test-market".parse().unwrap(),
        state: m::FeedState::Ready,
        last_sync_at: Some(time),
    });
    let value = json::record_value(&feed).unwrap();
    assert_eq!(value["last_sync_at"], "2026-11-09T16:00:00.123Z");
    assert!(value.get("last_sync_ms").is_none());
    assert!(json::parse_sources(br#"[{"type":"order_removed"}]"#, &catalog()).is_err());
}

#[test]
fn order_metadata_is_opaque_strict_and_omitted_when_empty() {
    let mut value = record_values()
        .into_iter()
        .find(|value| value["type"] == "order")
        .unwrap();
    let meta = json!({"url":"https://example.invalid/listing", "source.label":"", "observed_at":"source-local-time", "quantity":"unknown"});
    value["meta"] = meta.clone();
    let record = decode(value.clone()).unwrap().remove(0);
    assert_eq!(json::record_value(&record).unwrap()["meta"], meta);
    let m::Record::Order(order) = &record else {
        unreachable!()
    };
    assert_eq!(order.meta["observed_at"], "source-local-time");
    for bad in [
        Value::Null,
        json!([]),
        json!({"key":12}),
        json!({"key":false}),
        json!({"key":{}}),
        json!({"":"value"}),
    ] {
        let mut invalid = value.clone();
        invalid["meta"] = bad;
        assert!(decode(invalid).is_err());
    }
    let mut raw = value.clone();
    raw["meta"] = "__META__".into();
    let raw = serde_json::to_string(&vec![raw])
        .unwrap()
        .replace("\"__META__\"", r#"{"key":"a","key":"b"}"#);
    assert!(json::parse_records(raw.as_bytes(), &catalog()).is_err());
    let mut old = value.clone();
    old["url"] = "https://example.invalid/old".into();
    assert!(decode(old).is_err());
    value["meta"] = json!({});
    assert!(
        json::record_value(&decode(value).unwrap().remove(0))
            .unwrap()
            .get("meta")
            .is_none()
    );
}
