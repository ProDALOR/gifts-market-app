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

#[test]
fn restored_state_matches_live_indexes_through_updates_and_pinned_reads() {
    use crate::{query::Scope, types::Sort};
    let rows = crate::benchmarks::fixture(512);
    let mut live = State::default();
    live.apply(&rows).unwrap();
    let mut restored = State::default();
    // CURRENT is unique and sorted by identity, irrespective of input order.
    for (key, entity) in live.range(None, None) {
        restored
            .insert_restored(key.clone(), entity.as_ref().clone())
            .unwrap();
    }
    let pinned = restored.clone();
    let verify = |a: &State, b: &State| {
        assert!(a.range(None, None).eq(b.range(None, None)));
        assert_eq!(a.books().page(None, 100), b.books().page(None, 100));
        for collection in ["gift0", "gift1", "gift.moved"] {
            for currency in ["GRAM", "XTR"] {
                for modes in [vec![1], vec![2], vec![1, 2]] {
                    for attributes in [
                        BTreeMap::new(),
                        [(
                            "tg.model".into(),
                            w::AttributePredicate {
                                predicate: Some(w::attribute_predicate::Predicate::Eq(
                                    w::AttributeValue {
                                        value: Some(w::attribute_value::Value::OptionId(
                                            "model0".into(),
                                        )),
                                    },
                                )),
                            },
                        )]
                        .into(),
                    ] {
                        let scope = Scope::try_from(w::Scope {
                            item_type_id: Some(collection.into()),
                            currency_id: Some(currency.into()),
                            modes: modes.clone(),
                            attributes,
                            ..Default::default()
                        })
                        .unwrap();
                        for sort in [
                            Sort::PriceAsc,
                            Sort::PriceDesc,
                            Sort::TimeAsc,
                            Sort::TimeDesc,
                            Sort::NumberAsc,
                            Sort::NumberDesc,
                        ] {
                            let a: Vec<_> = a.books().ranked(&scope, None, sort).collect();
                            let b: Vec<_> = b.books().ranked(&scope, None, sort).collect();
                            assert_eq!(a, b);
                        }
                    }
                }
            }
        }
    };
    verify(&live, &restored);
    let original = live.clone();
    let orders: Vec<_> = live
        .range(None, Some(EntityKind::Order))
        .take(100)
        .map(|(_, e)| e.as_ref().clone())
        .collect();
    for (i, mut entity) in orders.into_iter().enumerate() {
        let key = entity_key(&entity).unwrap();
        if i % 5 == 0 {
            live.remove(&key).unwrap();
            restored.remove(&key).unwrap();
            continue;
        }
        if let Some(E::Order(order)) = &mut entity.entity {
            match i % 5 {
                1 => order.price = "999.5".into(),
                2 => {
                    order.mode = if order.mode == 1 { 2 } else { 1 };
                    order.book.as_mut().unwrap().item_type_id = "gift.moved".into();
                }
                3 => order.attributes.clear(),
                _ => {
                    order.meta.insert("changed".into(), "same rank".into());
                }
            }
        }
        live.upsert(entity.clone()).unwrap();
        restored.upsert(entity).unwrap();
    }
    verify(&live, &restored);
    verify(&original, &pinned);
}
