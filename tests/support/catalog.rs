use market_view as m;

pub fn catalog(feed: &str) -> m::Catalog {
    let mut definition = m::CatalogDefinition {
        venues: vec![m::VenueDefinition {
            id: "test-market".parse().unwrap(),
            name: "Test market".into(),
        }],
        currencies: vec![m::CurrencyDefinition {
            id: "TON".parse().unwrap(),
            name: "Toncoin".into(),
            decimals: 9,
        }],
        feeds: vec![m::FeedDefinition {
            id: feed.parse().unwrap(),
            venue_id: "test-market".parse().unwrap(),
        }],
        ..Default::default()
    };
    for (id, value_type, minimum, maximum, max_length) in [
        ("model", m::AttributeType::Enum, None, None, None),
        ("pattern", m::AttributeType::Enum, None, None, None),
        ("backdrop", m::AttributeType::Enum, None, None, None),
        (
            "serial-number",
            m::AttributeType::Integer,
            Some(m::AttributeValue::Integer(1)),
            None,
            None,
        ),
        (
            "wear",
            m::AttributeType::Decimal,
            Some(m::AttributeValue::decimal("0").unwrap()),
            Some(m::AttributeValue::decimal("1").unwrap()),
            None,
        ),
        (
            "pattern-seed",
            m::AttributeType::Integer,
            Some(m::AttributeValue::Integer(0)),
            Some(m::AttributeValue::Integer(1000)),
            None,
        ),
        (
            "special-variant",
            m::AttributeType::Boolean,
            None,
            None,
            None,
        ),
        ("custom-name", m::AttributeType::Text, None, None, Some(128)),
        (
            "available-at",
            m::AttributeType::Timestamp,
            None,
            None,
            None,
        ),
    ] {
        definition.attributes.push(m::AttributeDefinition {
            id: id.parse().unwrap(),
            name: id.into(),
            value_type,
            minimum,
            maximum,
            max_length,
        });
    }
    for (id, attribute, native) in [
        ("test-item.model.gold", "model", "model-1"),
        ("test-item.model.silver", "model", "model-2"),
        ("test-item.pattern.stars", "pattern", "pattern-1"),
        ("test-item.backdrop.blue", "backdrop", "backdrop-1"),
    ] {
        definition
            .attribute_values
            .push(m::AttributeValueDefinition {
                id: id.parse().unwrap(),
                attribute_id: attribute.parse().unwrap(),
                name: id.into(),
                aliases: vec![m::ExternalId {
                    namespace: "test-market".into(),
                    id: native.into(),
                }],
                meta: Default::default(),
            });
    }
    for (id, step, attributes) in [
        (
            "test-item",
            "1",
            vec![
                "model",
                "pattern",
                "backdrop",
                "serial-number",
                "available-at",
            ],
        ),
        (
            "test-skin",
            "1",
            vec![
                "wear",
                "pattern-seed",
                "special-variant",
                "custom-name",
                "available-at",
            ],
        ),
        (
            "test-divisible",
            "0.0001",
            vec!["custom-name", "available-at"],
        ),
    ] {
        definition.item_types.push(m::ItemTypeDefinition {
            id: id.parse().unwrap(),
            name: id.into(),
            quantity_step: step.parse().unwrap(),
            aliases: if id == "test-item" {
                vec![m::ExternalId {
                    namespace: "test-market".into(),
                    id: "item-type-1".into(),
                }]
            } else {
                Vec::new()
            },
            attributes: attributes
                .into_iter()
                .map(|id| (id.parse().unwrap(), m::AttributeRule::default()))
                .collect(),
        });
    }
    m::Catalog::from_entries(definition.into_entries()).unwrap()
}
