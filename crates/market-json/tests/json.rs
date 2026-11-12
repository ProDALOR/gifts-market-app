#[path = "../../../tests/support/mod.rs"]
mod support;
use market_json as json;
use market_view as m;
use serde_json::json;
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

#[test]
fn responses_are_flat_and_keep_exact_attribute_values() {
    let records = support::records();
    assert_eq!(records.len(), 9);
    for record in &records {
        let value = json::record_value(record).unwrap();
        assert!(value.get("data").is_none());
        assert!(value.get("amounts").is_none());
        assert!(value.get("observed_at_ms").is_none());
        assert!(value["observed_at"].is_string());
        if value["type"] == "order" {
            assert!(value["price"]["amount"].is_string());
            assert!(value["quantity"].is_string());
        }
    }
    let skin = records
        .iter()
        .find_map(|r| match r {
            m::Record::Item(v) if v.item_type_id.as_str() == "test-skin" => Some(v),
            _ => None,
        })
        .unwrap();
    let value = json::item_value(skin).unwrap();
    assert_eq!(value["attributes"]["wear"], "0.0342");
    assert_eq!(value["attributes"]["pattern-seed"], "661");
    assert_eq!(value["attributes"]["special-variant"], false);
    for source in support::sources() {
        let value = json::source_value(&source).unwrap();
        assert!(value.get("data").is_none());
        assert!(value.get("amounts").is_none());
        assert!(value["observed_at"].is_string());
    }
}

#[test]
fn query_scalars_use_declared_types_without_guessing_or_rounding() {
    use m::AttributeType as T;
    for (kind, raw, expected) in [
        (
            T::Decimal,
            json!("0.0000000000000000000000000001"),
            json!("0.0000000000000000000000000001"),
        ),
        (
            T::Integer,
            json!(i64::MAX.to_string()),
            json!(i64::MAX.to_string()),
        ),
        (T::Boolean, json!(false), json!(false)),
        (T::Text, json!(""), json!("")),
        (
            T::Timestamp,
            json!("2026-11-09T18:00:00+02:00"),
            json!("2026-11-09T16:00:00Z"),
        ),
        (
            T::Enum,
            json!("collection.model.gold"),
            json!("collection.model.gold"),
        ),
    ] {
        let value = json::decode_scalar(kind, serde_json::from_value(raw).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(json::Scalar::from(&value)).unwrap(),
            expected
        );
    }
    for (kind, raw) in [
        (T::Decimal, json!(true)),
        (T::Decimal, json!("0.00000000000000000000000000001")),
        (T::Integer, json!("9223372036854775808")),
        (T::Boolean, json!("false")),
        (T::Timestamp, json!("2026-11-09T16:00:00")),
    ] {
        assert!(json::decode_scalar(kind, serde_json::from_value(raw).unwrap()).is_err());
    }
    for raw in [
        r#"{"wear":0.03}"#,
        r#"{"wear":null}"#,
        r#"{"wear":{"type":"decimal","value":"0.03"}}"#,
        r#"{"wear":"0.03","wear":"0.04"}"#,
        r#"{"":"value"}"#,
    ] {
        assert!(json::parse_raw_attributes(raw.as_bytes()).is_err(), "{raw}");
    }
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
}

#[test]
fn order_metadata_is_opaque_and_omitted_when_empty() {
    let mut order = support::records()
        .into_iter()
        .find_map(|r| match r {
            m::Record::Order(v) => Some(v),
            _ => None,
        })
        .unwrap();
    order.data.meta.clear();
    assert!(json::order_value(&order).unwrap().get("meta").is_none());
    order.data.meta = [
        ("url".into(), "https://example.invalid/listing".into()),
        ("observed_at".into(), "source-local-time".into()),
    ]
    .into();
    let value = json::order_value(&order).unwrap();
    assert_eq!(value["meta"]["observed_at"], "source-local-time");
    assert_eq!(value["meta"]["url"], "https://example.invalid/listing");
}
