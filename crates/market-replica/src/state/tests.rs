use super::*;
use crate::test_support::{catalog, entity, order};
use std::collections::BTreeMap;
use w::entity::Entity as E;

#[test]
fn partition_owned_orders_preserve_global_identity_ranges_and_kind_boundaries() {
    let mut state = State::default();
    state.apply(&catalog()).unwrap();
    let feed = entity(E::Feed(w::Feed {
        id: "feed".into(),
        ..Default::default()
    }));
    state.upsert(feed).unwrap();
    let metadata = state.metadata.clone();
    for (venue, id, currency, mode) in [
        ("a", "same", "GRAM", 1),
        ("b", "same", "XTR", 2),
        ("a", "z", "XTR", 2),
        ("b", "a", "GRAM", 1),
    ] {
        let mut value = order(id, "1", None);
        value.book.as_mut().unwrap().venue_id = venue.into();
        value.book.as_mut().unwrap().currency_id = currency.into();
        value.mode = mode;
        state.upsert(entity(E::Order(value))).unwrap();
        assert!(state.metadata.ptr_eq(&metadata));
    }
    assert!(
        state
            .metadata
            .keys()
            .all(|key| key.kind != EntityKind::Order)
    );
    let all: BTreeMap<_, _> = state
        .metadata
        .iter()
        .map(|(key, record)| (key.clone(), record.clone()))
        .chain(
            [("a", "same"), ("b", "same"), ("a", "z"), ("b", "a")].map(|(venue, id)| {
                let key = Key::order(venue.into(), id.into());
                let record = state.entity(&key).unwrap().clone();
                (key, record)
            }),
        )
        .collect();
    let mut boundaries: Vec<_> = all.keys().cloned().map(Some).collect();
    boundaries.extend([
        None,
        Some(Key::order("a".into(), "missing".into())),
        Some(Key::order("z".into(), "missing".into())),
    ]);
    for kind in [
        None,
        Some(EntityKind::Venue),
        Some(EntityKind::Currency),
        Some(EntityKind::ItemType),
        Some(EntityKind::Attribute),
        Some(EntityKind::AttributeOption),
        Some(EntityKind::Feed),
        Some(EntityKind::Order),
    ] {
        for after in &boundaries {
            if kind.is_some() && after.as_ref().is_some_and(|key| Some(key.kind) != kind) {
                continue;
            }
            let expected: Vec<_> = all
                .iter()
                .filter(|(key, _)| {
                    kind.is_none_or(|kind| key.kind == kind)
                        && after.as_ref().is_none_or(|after| *key > after)
                })
                .map(|(key, record)| (key, record.as_ref()))
                .collect();
            let actual: Vec<_> = state
                .range(after.as_ref(), kind)
                .map(|(key, record)| (key, record.as_ref()))
                .collect();
            assert_eq!(actual, expected, "kind={kind:?}, after={after:?}");
        }
    }
    assert!(
        state
            .entity(&Key::order("missing".into(), "same".into()))
            .is_none()
    );
}
