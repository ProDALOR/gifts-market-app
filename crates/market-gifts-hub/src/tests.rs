use super::lookup::{collection_matches, model_matches};
use super::*;
use market_gifts::ModelLookup;
use market_gifts::{MediaTarget, TelegramDocumentId, TelegramGiftId};
use market_hub_protocol::wire as w;
fn option(namespace: &str, document: &str) -> w::AttributeOption {
    w::AttributeOption {
        id: "model/one".into(),
        attribute_id: MODEL.into(),
        name: "Model".into(),
        aliases: vec![w::ExternalId {
            namespace: namespace.into(),
            id: document.into(),
        }],
        ..Default::default()
    }
}
#[test]
fn choice_rarity_uses_numeric_metadata_for_every_trait_kind() {
    for kind in [MODEL, BACKDROP, PATTERN] {
        for value in ["0", "5", "10", "1000", "-1", "1001", "5.5", "rare", ""] {
            let mut option = option("tg", "1");
            option.attribute_id = kind.into();
            option.meta.insert("rarity.permille".into(), value.into());
            let rarity = match choice(option).unwrap().unwrap() {
                Choice::Model(value) => value.rarity,
                Choice::Backdrop(value) => value.rarity,
                Choice::Pattern(value) => value.rarity,
            };
            let expected = value.parse::<u16>().ok().filter(|v| *v <= 1000);
            assert_eq!(
                rarity,
                expected.map(|v| market_gifts::GiftRarity::Percentage {
                    permille: v.try_into().unwrap()
                }),
                "{kind}: {value}"
            );
        }
    }
    for (label, expected) in [
        ("uncommon", Some(market_gifts::GiftRarity::Uncommon)),
        ("rare", Some(market_gifts::GiftRarity::Rare)),
        ("epic", Some(market_gifts::GiftRarity::Epic)),
        ("legendary", Some(market_gifts::GiftRarity::Legendary)),
        ("unknown", None),
    ] {
        for kind in [MODEL, BACKDROP, PATTERN] {
            let mut label_only = option("tg", "1");
            label_only.attribute_id = kind.into();
            label_only.meta.insert("rarity.label".into(), label.into());
            let rarity = match choice(label_only).unwrap().unwrap() {
                Choice::Model(value) => value.rarity,
                Choice::Backdrop(value) => value.rarity,
                Choice::Pattern(value) => value.rarity,
            };
            assert_eq!(rarity, expected, "{kind}: {label}");
        }
    }
}
#[test]
fn telegram_aliases_and_logical_media_are_domain_rules_not_http_routes() {
    let model = option("tg", "9007199254740993");
    assert_eq!(document(&model).unwrap().get(), 9007199254740993);
    assert_eq!(
        media(&model).unwrap().target,
        MediaTarget::Model("model/one".into())
    );
    let encoded = serde_json::to_string(&media(&model)).unwrap();
    assert!(!encoded.contains("url"));
    assert!(!encoded.contains("/media/"));
    assert!(media(&option("other", "1")).is_none());
    assert!(media(&option("tg", "-1")).is_none());
    assert!(media(&option("tg", "invalid")).is_none());
    let mut pattern = model.clone();
    pattern.attribute_id = PATTERN.into();
    assert!(media_document(&pattern, &MediaTarget::Model("model/one".into())).is_none());
    assert!(media_document(&pattern, &MediaTarget::Pattern("model/one".into())).is_some());
    assert!(media_document(&pattern, &MediaTarget::Pattern("other".into())).is_none());
}
#[test]
fn gift_identity_and_model_lookups_share_strict_telegram_namespace_rules() {
    let mut item = w::ItemType {
        id: "collection".into(),
        name: "Gift".into(),
        aliases: vec![w::ExternalId {
            namespace: "tg".into(),
            id: "123".into(),
        }],
        ..Default::default()
    };
    let lookup = ModelLookup::Telegram {
        gift_id: TelegramGiftId::new(123).unwrap(),
        document_id: TelegramDocumentId::new(9007199254740993).unwrap(),
    };
    assert!(is_gift(&item));
    assert!(collection_matches(&lookup, &item));
    assert!(model_matches(&lookup, &option("tg", "9007199254740993")));
    assert!(!model_matches(
        &lookup,
        &option("other", "9007199254740993")
    ));
    item.aliases[0].namespace = "other".into();
    assert!(!is_gift(&item));
    assert!(!collection_matches(&lookup, &item));
}
#[test]
fn collection_rules_validate_typed_ids_and_distinguish_empty_from_unrestricted() {
    let item = w::ItemType {
        attributes: [
            (MODEL.into(), w::AttributeRule::default()),
            (
                PATTERN.into(),
                w::AttributeRule {
                    required: false,
                    allowed_values: Some(w::AllowedAttributeValues { ids: vec![] }),
                },
            ),
        ]
        .into(),
        ..Default::default()
    };
    let rules = CollectionChoices::new(&item).unwrap();
    assert_eq!(rules.unrestricted_attributes(), vec![MODEL]);
    assert_eq!(rules.model_option_ids(), None);
    assert!(rules.accepts(&option("tg", "1")));
    let mut pattern = option("tg", "1");
    pattern.attribute_id = PATTERN.into();
    assert!(!rules.accepts(&pattern));
    let mut invalid = item;
    invalid.attributes.get_mut(MODEL).unwrap().allowed_values = Some(w::AllowedAttributeValues {
        ids: vec![" ".into()],
    });
    assert!(CollectionChoices::new(&invalid).is_err());
}

#[test]
fn model_lookup_rejects_wrong_kinds_and_reports_missing_or_ambiguous_names() {
    use market_gifts::ModelLookupError;
    let lookup = ModelLookup::Name {
        collection: "Gift".into(),
        model: "Model".into(),
    };
    let collection = w::ItemType {
        id: "collection".into(),
        name: "Gift".into(),
        attributes: [(MODEL.into(), w::AttributeRule::default())].into(),
        ..Default::default()
    };
    assert_eq!(
        find_collection(&lookup, std::iter::once(&collection))
            .unwrap()
            .id,
        "collection"
    );
    assert_eq!(
        find_collection(&lookup, [&collection, &collection].into_iter()).unwrap_err(),
        ModelLookupError::AmbiguousCollection
    );
    assert_eq!(
        find_collection(&lookup, std::iter::empty()).unwrap_err(),
        ModelLookupError::UnknownCollection
    );
    let model = option("tg", "1");
    let mut pattern = model.clone();
    pattern.attribute_id = PATTERN.into();
    assert_eq!(
        find_model(&lookup, [&pattern, &model].into_iter())
            .unwrap()
            .id,
        model.id
    );
    assert_eq!(
        find_model(&lookup, std::iter::once(&pattern)).unwrap_err(),
        ModelLookupError::UnknownModel
    );
    assert_eq!(
        find_model(&lookup, [&model, &model].into_iter()).unwrap_err(),
        ModelLookupError::AmbiguousModel
    );
    let identity = model_identity(&collection, &model).unwrap();
    assert_eq!(identity.collection_id.as_str(), "collection");
    assert_eq!(identity.model_id.as_str(), "model/one");
}

#[test]
fn choice_rules_keep_trait_membership_separate_and_normalize_requested_ids() {
    let item = w::ItemType {
        attributes: [
            (
                MODEL.into(),
                w::AttributeRule {
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec!["b".into(), "a".into(), "a".into()],
                    }),
                    ..Default::default()
                },
            ),
            (
                BACKDROP.into(),
                w::AttributeRule {
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec!["backdrop".into()],
                    }),
                    ..Default::default()
                },
            ),
        ]
        .into(),
        ..Default::default()
    };
    let rules = CollectionChoices::new(&item).unwrap();
    assert_eq!(rules.model_option_ids(), Some(vec!["a".into(), "b".into()]));
    assert_eq!(rules.option_ids(), vec!["a", "b", "backdrop"]);
    let mut model = option("tg", "1");
    model.id = "a".into();
    assert!(rules.accepts(&model));
    model.attribute_id = BACKDROP.into();
    assert!(!rules.accepts(&model));
    model.id = "backdrop".into();
    assert!(rules.accepts(&model));
    assert_eq!(
        CollectionChoices::new(&w::ItemType::default())
            .unwrap()
            .model_option_ids(),
        Some(vec![])
    );
}

#[test]
fn collection_slugs_are_explicit_stable_and_validated() {
    let mut item = w::ItemType {
        id: "collection".into(),
        name: "Plush Pepe".into(),
        attributes: [(MODEL.into(), w::AttributeRule::default())].into(),
        ..Default::default()
    };
    assert!(collection(item.clone()).unwrap().slug.is_none());
    item.aliases.push(w::ExternalId {
        namespace: COLLECTION_SLUG_NAMESPACE.into(),
        id: "PlushPepe".into(),
    });
    let original = collection(item.clone()).unwrap();
    item.name = "A completely different display name".into();
    let renamed = collection(item.clone()).unwrap();
    assert_eq!(original.slug, renamed.slug);
    assert_eq!(renamed.slug.unwrap().as_str(), "PlushPepe");
    item.aliases.push(w::ExternalId {
        namespace: COLLECTION_SLUG_NAMESPACE.into(),
        id: "plushpepe".into(),
    });
    assert_eq!(
        collection_slug(&item).unwrap().unwrap().lookup_key(),
        "plushpepe"
    );
    item.aliases[1].id = "OtherGift".into();
    assert!(collection(item.clone()).is_err());
    item.aliases.truncate(1);
    item.aliases[0].id = "Plush Pepe".into();
    assert!(collection(item).is_err());
}
