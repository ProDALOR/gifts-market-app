use super::*;
use market_gifts::*;

fn subjects() -> Vec<Arc<ResolvedTarget>> {
    vec![
        ResolvedTarget::Selection {
            collection_id: "gift".into(),
            criteria: market_gifts::GiftCriteria::default(),
        },
        ResolvedTarget::Selection {
            collection_id: "gift".into(),
            criteria: market_gifts::GiftCriteria {
                models: vec!["model".into()],
                ..Default::default()
            },
        },
        ResolvedTarget::Selection {
            collection_id: "gift".into(),
            criteria: market_gifts::GiftCriteria {
                backdrops: vec!["backdrop".into()],
                ..Default::default()
            },
        },
        ResolvedTarget::Selection {
            collection_id: "gift".into(),
            criteria: market_gifts::GiftCriteria {
                models: vec!["model".into()],
                backdrops: vec!["backdrop".into()],
                ..Default::default()
            },
        },
        ResolvedTarget::Selection {
            collection_id: "gift".into(),
            criteria: GiftCriteria {
                models: vec!["m1".into(), "m2".into()],
                backdrops: vec!["b".into()],
                patterns: vec!["p".into()],
                number: Some(GiftNumberRange {
                    min: Some(GiftNumber::new(10).unwrap()),
                    max: Some(GiftNumber::new(20).unwrap()),
                }),
            },
        },
        ResolvedTarget::GiftAttributes(GiftAttributes {
            number: None,
            collection_id: "gift".into(),
            traits: GiftTraits {
                model: None,
                backdrop: None,
                pattern: Some("symbol".into()),
            },
        }),
        ResolvedTarget::Gift {
            gift: CollectibleGift {
                id: "gift-123".into(),
                collection_id: "gift".into(),
                number: Some(GiftNumber::new(123).unwrap()),
                traits: GiftTraits {
                    model: None,
                    backdrop: None,
                    pattern: Some("symbol".into()),
                },
            },
            slug: Some(GiftSlug::new("Gift-123").unwrap()),
        },
    ]
    .into_iter()
    .map(Arc::new)
    .collect()
}
#[test]
fn selection_contract_shares_all_resolved_subjects_and_retains_joint_conditions() {
    for target in subjects() {
        let input = SelectionInput::resolve(target.clone(), None).unwrap();
        assert!(std::ptr::eq(input.target(), target.as_ref()));
        assert!(SelectionInput::CONTRACT.targets().contains(&target.kind()));
    }
    assert!(!SelectionInput::CONTRACT.requires_period());
}
#[test]
fn execution_input_always_has_a_valid_explicit_half_open_period() {
    let from = "2026-01-01T00:00:00Z".parse().unwrap();
    let until = "2026-02-01T00:00:00Z".parse().unwrap();
    let period = Period {
        from: Some(from),
        until: Some(until),
    };
    for target in subjects() {
        let input = ExecutionInput::resolve(
            target.clone(),
            Some(ExecutionPeriod::resolve(Some(&period)).unwrap()),
        )
        .unwrap();
        assert!(std::ptr::eq(input.selection().target(), target.as_ref()));
        assert_eq!(input.period().from(), from);
        assert_eq!(input.period().until(), until);
        assert_eq!(input.period().as_period(), period);
        assert!(ExecutionInput::CONTRACT.targets().contains(&target.kind()));
    }
    let target = subjects().remove(0);
    for period in [
        None,
        Some(Period::default()),
        Some(Period {
            from: Some(from),
            until: None,
        }),
        Some(Period {
            from: None,
            until: Some(until),
        }),
        Some(Period {
            from: Some(until),
            until: Some(from),
        }),
        Some(Period {
            from: Some(from),
            until: Some(from),
        }),
        Some(Period {
            from: Some(from),
            until: Some("2099-01-01T00:00:00Z".parse().unwrap()),
        }),
    ] {
        let error = ExecutionPeriod::resolve(period.as_ref()).unwrap_err();
        assert_eq!(error.code(), crate::ValuationErrorCode::InvalidQuery);
    }
    assert!(ExecutionInput::resolve(target, None).is_err());
    assert!(ExecutionInput::CONTRACT.requires_period());
}
#[test]
fn gift_contract_rejects_other_subjects_and_preserves_optional_facts() {
    for target in subjects() {
        let result = GiftAttributes::resolve(target.clone(), None);
        if let Some(gift) = target.gift() {
            let input = result.unwrap();
            assert_eq!(input.collection_id, gift.collection_id);
            assert!(input.traits.model.is_none());
            assert!(input.traits.backdrop.is_none());
            assert_eq!(input.traits.pattern.as_ref().unwrap().as_str(), "symbol");
        } else if let ResolvedTarget::GiftAttributes(attributes) = target.as_ref() {
            assert_eq!(&result.unwrap(), attributes);
        } else {
            assert_eq!(
                result.unwrap_err().code(),
                crate::ValuationErrorCode::UnsupportedTarget
            );
        }
    }
    assert_eq!(
        GiftAttributes::CONTRACT.targets(),
        &[
            ValuationTargetKind::Gift,
            ValuationTargetKind::GiftAttributes
        ]
    );
    assert!(!GiftAttributes::CONTRACT.requires_period());
}
