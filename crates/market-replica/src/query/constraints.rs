//! Bounded scalar constraint relations. Complex predicate trees are intentionally not solved.
use market_hub_protocol::wire as w;
use w::attribute_predicate::Predicate as P;
use w::attribute_value::Value as V;

pub(super) fn supported(predicate: &w::AttributePredicate) -> bool {
    let scalar = |v: &w::AttributeValue| {
        v.value
            .as_ref()
            .is_some_and(|v| !matches!(v, V::List(_) | V::Object(_)))
    };
    match predicate.predicate.as_ref() {
        Some(P::Eq(v)) => scalar(v),
        Some(P::OneOf(v)) => !v.values.is_empty() && v.values.iter().all(scalar),
        Some(P::Range(r)) => r
            .min
            .as_ref()
            .or(r.max.as_ref())
            .is_some_and(|v| super::compare(v, v).is_some()),
        _ => false,
    }
}

pub(super) fn matches(
    requested: &w::AttributePredicate,
    accepted: &w::AttributePredicate,
    mode: crate::AttributeMatch,
) -> bool {
    relation(requested, accepted, mode == crate::AttributeMatch::Exact)
        && (mode != crate::AttributeMatch::Exact || relation(accepted, requested, true))
}

/// Exact constraints contain one another; overlap only needs one shared value.
fn relation(
    requested: &w::AttributePredicate,
    accepted: &w::AttributePredicate,
    covers: bool,
) -> bool {
    match requested.predicate.as_ref() {
        Some(P::Eq(v)) => super::matches(accepted, v),
        Some(P::OneOf(values)) => {
            if covers {
                values.values.iter().all(|v| super::matches(accepted, v))
            } else {
                values.values.iter().any(|v| super::matches(accepted, v))
            }
        }
        Some(P::Range(wanted)) => match accepted.predicate.as_ref() {
            Some(P::Range(offered)) => {
                if covers {
                    lower_covers(offered.min.as_ref(), wanted.min.as_ref())
                        && upper_covers(offered.max.as_ref(), wanted.max.as_ref())
                } else {
                    ordered(wanted.min.as_ref(), offered.max.as_ref())
                        && ordered(offered.min.as_ref(), wanted.max.as_ref())
                }
            }
            Some(P::Eq(v)) => finite_range(requested, wanted, std::slice::from_ref(v), covers),
            Some(P::OneOf(v)) => finite_range(requested, wanted, &v.values, covers),
            _ => false,
        },
        _ => false,
    }
}
fn ordered(lower: Option<&w::AttributeValue>, upper: Option<&w::AttributeValue>) -> bool {
    lower
        .zip(upper)
        .is_none_or(|(a, b)| super::compare(a, b).is_some_and(|v| !v.is_gt()))
}
fn lower_covers(offered: Option<&w::AttributeValue>, wanted: Option<&w::AttributeValue>) -> bool {
    offered.is_none() || wanted.is_some_and(|v| ordered(offered, Some(v)))
}
fn upper_covers(offered: Option<&w::AttributeValue>, wanted: Option<&w::AttributeValue>) -> bool {
    offered.is_none() || wanted.is_some_and(|v| ordered(Some(v), offered))
}
fn finite_range(
    predicate: &w::AttributePredicate,
    range: &w::AttributeRange,
    values: &[w::AttributeValue],
    covers: bool,
) -> bool {
    if !covers {
        return values.iter().any(|v| super::matches(predicate, v));
    }
    let Some((min, max)) = range.min.as_ref().zip(range.max.as_ref()) else {
        return false;
    };
    if super::equal(min, max) {
        return values.iter().any(|v| super::equal(min, v));
    }
    // Integer and millisecond ranges are discrete. Count unique members without expanding a range.
    let count = match (&min.value, &max.value) {
        (Some(V::Integer(a)), Some(V::Integer(b)))
        | (Some(V::TimestampMs(a)), Some(V::TimestampMs(b))) => i128::from(*b) - i128::from(*a) + 1,
        _ => return false,
    };
    if count <= 0 || count > values.len() as i128 {
        return false;
    }
    let unique: std::collections::BTreeSet<_> = values
        .iter()
        .filter(|v| super::matches(predicate, v))
        .map(prost::Message::encode_to_vec)
        .collect();
    unique.len() as i128 == count
}

#[cfg(test)]
mod tests {
    use super::*;
    fn integer(n: i64) -> w::AttributeValue {
        w::AttributeValue {
            value: Some(V::Integer(n)),
        }
    }
    fn point(n: i64) -> w::AttributePredicate {
        w::AttributePredicate {
            predicate: Some(P::Eq(integer(n))),
        }
    }
    fn set(values: &[i64]) -> w::AttributePredicate {
        w::AttributePredicate {
            predicate: Some(P::OneOf(w::AttributeValues {
                values: values.iter().map(|n| integer(*n)).collect(),
            })),
        }
    }
    fn range(min: Option<i64>, max: Option<i64>) -> w::AttributePredicate {
        w::AttributePredicate {
            predicate: Some(P::Range(w::AttributeRange {
                min: min.map(integer),
                max: max.map(integer),
            })),
        }
    }
    #[test]
    fn any_overlaps_while_all_requires_exactly_the_selected_values() {
        let wanted = set(&[1, 2]);
        for (accepted, any, all) in [
            (point(1), true, false),
            (set(&[1, 2]), true, true),
            (set(&[1, 2, 3]), true, false),
            (point(3), false, false),
            (range(Some(0), Some(2)), true, false),
            (range(Some(1), Some(2)), true, true),
        ] {
            assert_eq!(
                matches(&wanted, &accepted, crate::AttributeMatch::Overlap),
                any
            );
            assert_eq!(
                matches(&wanted, &accepted, crate::AttributeMatch::Exact),
                all
            );
        }
    }
    #[test]
    fn empty_exact_selections_keep_their_matching_policy_and_read_identity() {
        let request = w::ReadRequest {
            selection: Some(w::Selection {
                source: Some(w::selection::Source::Scope(w::Scope {
                    modes: vec![w::OrderMode::Bid as i32],
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        };
        let state = crate::state::State::default();
        let compile = |attribute_match| {
            crate::query::Filter::compile_with_options(
                &state,
                &request,
                crate::ReadOptions {
                    attribute_match,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let any = compile(crate::AttributeMatch::Overlap);
        let all = compile(crate::AttributeMatch::Exact);
        assert_ne!(any.fingerprint, all.fingerprint);
        assert_eq!(
            all.scope.unwrap().attribute_match,
            crate::AttributeMatch::Exact
        );
    }
    #[test]
    fn filtered_bid_options_are_local_and_have_distinct_read_identities() {
        use w::entity::Entity as E;
        let mut state = crate::state::State::default();
        let changes = [
            E::Attribute(w::Attribute {
                id: "value".into(),
                name: "Value".into(),
                value_type: 2,
                ..Default::default()
            }),
            E::ItemType(w::ItemType {
                id: "item".into(),
                name: "Item".into(),
                quantity_step: "1".into(),
                attributes: [("value".into(), Default::default())].into(),
                ..Default::default()
            }),
        ]
        .map(|entity| w::Change {
            change: Some(w::change::Change::Upsert(w::Entity {
                entity: Some(entity),
            })),
        });
        state.apply(&changes).unwrap();
        let request = w::ReadRequest {
            selection: Some(w::Selection {
                source: Some(w::selection::Source::Scope(w::Scope {
                    item_type_id: Some("item".into()),
                    modes: vec![w::OrderMode::Bid as i32],
                    attributes: [("value".into(), point(1))].into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        };
        // Upstream/default reads retain the original concrete-value semantics.
        assert!(crate::query::Filter::compile(&state, &request).is_err());
        let compile = |attribute_match| {
            crate::query::Filter::compile_with_options(
                &state,
                &request,
                crate::ReadOptions {
                    attribute_match,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let any = compile(crate::AttributeMatch::Overlap);
        let all = compile(crate::AttributeMatch::Exact);
        assert_ne!(any.fingerprint, all.fingerprint);
        assert_eq!(
            any.scope.unwrap().attribute_match,
            crate::AttributeMatch::Overlap
        );
        assert_eq!(
            all.scope.unwrap().attribute_match,
            crate::AttributeMatch::Exact
        );
    }
    #[test]
    fn inclusive_ranges_require_equal_bounds_for_exact_matching() {
        let wanted = range(Some(10), Some(20));
        for (accepted, any, all) in [
            (range(Some(20), Some(30)), true, false),
            (range(Some(21), None), false, false),
            (range(Some(5), Some(25)), true, false),
            (range(None, Some(20)), true, false),
            (range(Some(10), Some(20)), true, true),
            (point(15), true, false),
        ] {
            assert_eq!(
                matches(&wanted, &accepted, crate::AttributeMatch::Overlap),
                any
            );
            assert_eq!(
                matches(&wanted, &accepted, crate::AttributeMatch::Exact),
                all
            );
        }
        assert!(!matches(
            &range(None, Some(20)),
            &range(Some(0), Some(20)),
            crate::AttributeMatch::Exact
        ));
        assert!(!matches(
            &range(Some(10), None),
            &range(Some(5), None),
            crate::AttributeMatch::Exact
        ));
        assert!(matches(
            &range(Some(10), Some(10)),
            &point(10),
            crate::AttributeMatch::Exact
        ));
    }
    #[test]
    fn exact_finite_sets_equal_integer_ranges_without_gaps_or_extra_values() {
        let wanted = range(Some(1), Some(3));
        assert!(matches(
            &wanted,
            &set(&[3, 2, 1, 1]),
            crate::AttributeMatch::Exact
        ));
        assert!(!matches(
            &wanted,
            &set(&[0, 1, 2, 3]),
            crate::AttributeMatch::Exact
        ));
        assert!(matches(
            &point(1),
            &set(&[1, 1]),
            crate::AttributeMatch::Exact
        ));
        assert!(matches(
            &range(None, Some(3)),
            &range(None, Some(3)),
            crate::AttributeMatch::Exact
        ));
        assert!(!matches(
            &wanted,
            &set(&[1, 3, 3]),
            crate::AttributeMatch::Exact
        ));
        assert!(!matches(
            &range(Some(i64::MIN), Some(i64::MAX)),
            &set(&[1, 2]),
            crate::AttributeMatch::Exact
        ));
        assert!(!supported(&w::AttributePredicate {
            predicate: Some(P::AllOf(w::AttributePredicates {
                predicates: vec![point(1)]
            }))
        }));
    }
}
