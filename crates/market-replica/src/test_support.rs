use market_hub_protocol::wire as w;
use w::entity::Entity as E;
pub(super) fn entity(value: E) -> w::Entity {
    w::Entity {
        entity: Some(value),
    }
}
pub(super) fn upsert(value: w::Entity) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Upsert(value)),
    }
}
pub(super) fn catalog() -> Vec<w::Change> {
    vec![
        upsert(entity(E::Venue(w::Venue {
            id: "venue".into(),
            name: "Venue".into(),
        }))),
        upsert(entity(E::Currency(w::Currency {
            id: "GRAM".into(),
            name: "Gram".into(),
            decimals: 9,
        }))),
        upsert(entity(E::Currency(w::Currency {
            id: "XTR".into(),
            name: "Stars".into(),
            decimals: 9,
        }))),
        upsert(entity(E::ItemType(w::ItemType {
            id: "gift".into(),
            name: "Gift".into(),
            quantity_step: "1".into(),
            attributes: [("stickers".into(), w::AttributeRule::default())].into(),
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "stickers".into(),
            name: "Stickers".into(),
            value_type: 7,
            element: Some("sticker".into()),
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "sticker".into(),
            name: "Sticker".into(),
            value_type: 8,
            fields: [
                ("name".into(), w::AttributeRule::default()),
                ("wear".into(), w::AttributeRule::default()),
            ]
            .into(),
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "name".into(),
            name: "Name".into(),
            value_type: 5,
            ..Default::default()
        }))),
        upsert(entity(E::Attribute(w::Attribute {
            id: "wear".into(),
            name: "Wear".into(),
            value_type: 3,
            ..Default::default()
        }))),
    ]
}
pub(super) fn order(id: &str, price: &str, time: Option<i64>) -> w::Order {
    w::Order {
        id: id.into(),
        book: Some(w::BookKey {
            venue_id: "venue".into(),
            item_type_id: "gift".into(),
            currency_id: "GRAM".into(),
        }),
        mode: if time.is_some() { 3 } else { 1 },
        price: price.into(),
        quantity: "1".into(),
        observed_at_ms: 1000,
        executed_at_ms: time,
        ..Default::default()
    }
}
pub(super) fn read(mode: i32, history: bool) -> w::ReadRequest {
    w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(w::Scope {
                item_type_id: Some("gift".into()),
                currency_id: Some("GRAM".into()),
                modes: vec![if history { 3 } else { 1 }],
                ..Default::default()
            })),
            ..Default::default()
        }),
        mode,
        sort: if history { 0 } else { 1 },
        limit: 1,
        ..Default::default()
    }
}

pub(crate) fn order_entity(entity: &w::Entity) -> Option<&w::Order> {
    match entity.entity.as_ref()? {
        E::Order(order) => Some(order),
        _ => None,
    }
}

pub(crate) fn ids(page: &w::Page) -> Vec<String> {
    page.entities
        .iter()
        .map(|entity| order_entity(entity).unwrap().id.clone())
        .collect()
}
