#![allow(dead_code)]
//! Test-only builders; no persisted market data or runtime configuration is loaded.
use market_view as m;

pub fn records() -> Vec<m::Record> {
    let time = "2026-11-09T16:00:00Z".parse().unwrap();
    let item = |kind: &str, id: &str, attributes| {
        m::Record::Item(m::Item {
            feed_id: "source".parse().unwrap(),
            item_type_id: kind.parse().unwrap(),
            observed_at: time,
            data: m::ItemData {
                item_id: id.parse().unwrap(),
                attributes,
            },
        })
    };
    let order = |kind: &str, id: &str, price: &str, quantity: &str, selector| {
        m::Record::Order(m::Order {
            feed_id: "source".parse().unwrap(),
            item_type_id: kind.parse().unwrap(),
            observed_at: time,
            data: m::OrderData {
                order_id: id.parse().unwrap(),
                amounts: m::PriceQuantity {
                    price: m::Money::parse("TON".parse().unwrap(), price).unwrap(),
                    quantity: quantity.parse().unwrap(),
                },
                kind: selector,
                meta: Default::default(),
                expires_at: None,
            },
        })
    };
    let appearance: m::Attributes = [
        (
            "model".parse().unwrap(),
            m::AttributeValue::Enum("test-item.model.gold".parse().unwrap()),
        ),
        (
            "pattern".parse().unwrap(),
            m::AttributeValue::Enum("test-item.pattern.stars".parse().unwrap()),
        ),
        (
            "backdrop".parse().unwrap(),
            m::AttributeValue::Enum("test-item.backdrop.blue".parse().unwrap()),
        ),
        (
            "serial-number".parse().unwrap(),
            m::AttributeValue::Integer(1),
        ),
    ]
    .into();
    let skin: m::Attributes = [
        (
            "wear".parse().unwrap(),
            m::AttributeValue::decimal("0.0342").unwrap(),
        ),
        (
            "pattern-seed".parse().unwrap(),
            m::AttributeValue::Integer(661),
        ),
        (
            "special-variant".parse().unwrap(),
            m::AttributeValue::Boolean(false),
        ),
        (
            "custom-name".parse().unwrap(),
            m::AttributeValue::Text("Test".into()),
        ),
        (
            "available-at".parse().unwrap(),
            m::AttributeValue::Timestamp(time),
        ),
    ]
    .into();
    let mut records = vec![
        item("test-item", "telegram:123456789", appearance),
        order(
            "test-item",
            "listing-1",
            "12.50",
            "1",
            m::OrderKind::AskItem {
                item_id: "telegram:123456789".parse().unwrap(),
            },
        ),
        order(
            "test-item",
            "bid-1",
            "10",
            "3",
            m::OrderKind::BidAttributes {
                filter: [(
                    "model".parse().unwrap(),
                    [
                        m::AttributeValue::Enum("test-item.model.gold".parse().unwrap()),
                        m::AttributeValue::Enum("test-item.model.silver".parse().unwrap()),
                    ]
                    .into(),
                )]
                .into(),
            },
        ),
        item("test-skin", "inventory:skin-1", skin.clone()),
        order(
            "test-skin",
            "skin-bid",
            "20",
            "2",
            m::OrderKind::BidAttributes {
                filter: [
                    (
                        "wear".parse().unwrap(),
                        m::AttributePredicate::Range {
                            min: Some(m::AttributeValue::decimal("0").unwrap()),
                            max: Some(m::AttributeValue::decimal("0.035").unwrap()),
                        },
                    ),
                    (
                        "special-variant".parse().unwrap(),
                        m::AttributePredicate::Eq {
                            value: m::AttributeValue::Boolean(false),
                        },
                    ),
                ]
                .into(),
            },
        ),
        order(
            "test-skin",
            "skin-unit-ask",
            "25",
            "100",
            m::OrderKind::AskUnits { attributes: skin },
        ),
        item(
            "test-divisible",
            "inventory:divisible-1",
            Default::default(),
        ),
        order(
            "test-divisible",
            "fraction-ask",
            "100",
            "0.75",
            m::OrderKind::AskItem {
                item_id: "inventory:divisible-1".parse().unwrap(),
            },
        ),
        order(
            "test-divisible",
            "fraction-bid",
            "90",
            "0.25",
            m::OrderKind::BidItem {
                item_id: "inventory:divisible-1".parse().unwrap(),
            },
        ),
    ];
    if let m::Record::Order(order) = &mut records[1] {
        order.meta = [
            ("url".into(), "https://example.invalid/listing-1".into()),
            ("source.label".into(), "Featured listing".into()),
        ]
        .into();
    }
    records
}
pub fn sources() -> Vec<m::SourceEvent> {
    let time = m::MarketTimestamp::from_millis(1794240000000).unwrap();
    let executed = m::MarketTimestamp::from_millis(1794239999000).unwrap();
    vec![
        m::SourceEvent::Trade(m::Observed {
            feed_id: "source".parse().unwrap(),
            item_type_id: "test-divisible".parse().unwrap(),
            observed_at: time,
            data: m::TradeData {
                trade_id: "execution-1".parse().unwrap(),
                amounts: m::PriceQuantity {
                    price: m::Money::parse("TON".parse().unwrap(), "100").unwrap(),
                    quantity: "0.25".parse().unwrap(),
                },
                executed_at: executed,
                buy_order_id: None,
                sell_order_id: Some("fraction-ask".parse().unwrap()),
                kind: m::TradeKind::Item {
                    item_id: "example:divisible-1".parse().unwrap(),
                },
            },
        }),
        m::SourceEvent::OrderStatus(m::Observed {
            feed_id: "source".parse().unwrap(),
            item_type_id: "test-divisible".parse().unwrap(),
            observed_at: time,
            data: m::OrderStatusData {
                event_id: "status-1".parse().unwrap(),
                order: m::OrderRef {
                    order_id: "fraction-ask".parse().unwrap(),
                    side: m::OrderSide::Ask,
                },
                status: m::OrderStatus::PartiallyFilled,
                occurred_at: Some(executed),
            },
        }),
    ]
}
