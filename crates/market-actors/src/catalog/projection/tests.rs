use super::*;
use crate::catalog::cache::ProjectionCache;
use market_gifts::MediaTarget;
use market_gifts_hub::{BACKDROP, MODEL, PATTERN};
use market_hub_protocol::wire as w;
#[test]
fn collection_details_batch_large_ids_preserve_colors_and_reject_wrong_media_kinds() {
    let directory = tempfile::tempdir().unwrap();
    let mut writer =
        market_replica::ReplicaWriter::open(directory.path(), "gift-catalog-test").unwrap();
    let mut snapshot = writer.begin_snapshot().unwrap();
    let ids: Vec<_> = (0..701).map(|i| format!("model.{i}")).collect();
    let rule = |ids: Vec<String>| w::AttributeRule {
        required: true,
        allowed_values: Some(w::AllowedAttributeValues { ids }),
    };
    let mut rows = vec![
        w::entity::Entity::ItemType(w::ItemType {
            id: "collection".into(),
            name: "Gifts".into(),
            quantity_step: "1".into(),
            attributes: [
                (MODEL.into(), rule(ids.clone())),
                (BACKDROP.into(), rule(vec!["backdrop".into()])),
                (PATTERN.into(), rule(vec!["pattern".into()])),
            ]
            .into(),
            ..Default::default()
        }),
        w::entity::Entity::ItemType(w::ItemType {
            id: "not-a-gift".into(),
            name: "Commodity".into(),
            quantity_step: "0.1".into(),
            ..Default::default()
        }),
        w::entity::Entity::AttributeOption(w::AttributeOption {
            id: "backdrop".into(),
            name: "Lavender".into(),
            attribute_id: BACKDROP.into(),
            meta: [
                ("colors.center_color".into(), "#AABBCC".into()),
                ("colors.edge_color".into(), "#123456".into()),
                ("colors.pattern_color".into(), "#223344".into()),
            ]
            .into(),
            ..Default::default()
        }),
        w::entity::Entity::AttributeOption(w::AttributeOption {
            id: "pattern".into(),
            name: "Stars".into(),
            attribute_id: PATTERN.into(),
            aliases: vec![w::ExternalId {
                namespace: "tg".into(),
                id: "555".into(),
            }],
            ..Default::default()
        }),
    ];
    rows.extend(ids.into_iter().enumerate().map(|(i, id)| {
        w::entity::Entity::AttributeOption(w::AttributeOption {
            id,
            name: format!("Model {i}"),
            attribute_id: MODEL.into(),
            aliases: vec![w::ExternalId {
                namespace: "tg".into(),
                id: (1000 + i).to_string(),
            }],
            ..Default::default()
        })
    }));
    snapshot
        .append(
            &rows
                .into_iter()
                .map(|row| w::Change {
                    change: Some(w::change::Change::Upsert(w::Entity { entity: Some(row) })),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    writer
        .install_snapshot(snapshot.finish("catalog-checkpoint").unwrap())
        .unwrap();
    let catalog = ProjectionCache::default()
        .bootstrap(writer.reader())
        .unwrap();
    assert_eq!(catalog.collections.len(), 1);
    let preview = catalog.collections[0].preview.as_ref().unwrap();
    assert_eq!(preview.target, MediaTarget::Model("model.0".into()));
    let result = choices(&writer.reader().snapshot().unwrap(), "collection".into())
        .unwrap()
        .catalog;
    assert_eq!(result.models.len(), 701);
    assert_eq!(result.patterns.len(), 1);
    assert_eq!(
        result.backdrops[0]
            .palette
            .as_ref()
            .unwrap()
            .center
            .to_string(),
        "#aabbcc"
    );
    assert_eq!(
        result
            .models
            .iter()
            .find(|m| m.id.as_str() == "model.0")
            .unwrap()
            .media
            .as_ref()
            .unwrap()
            .target,
        MediaTarget::Model("model.0".into())
    );
    assert_eq!(
        media_document(writer.reader(), MediaTarget::Pattern("pattern".into()))
            .unwrap()
            .get(),
        555
    );
    assert!(media_document(writer.reader(), MediaTarget::Model("pattern".into())).is_err());
    assert!(choices(&writer.reader().snapshot().unwrap(), "not-a-gift".into()).is_err());
    assert_eq!(model_documents(writer.reader(), 1).unwrap().len(), 1);
    assert_eq!(model_documents(writer.reader(), 64).unwrap().len(), 64);
    assert_eq!(model_documents(writer.reader(), 80).unwrap().len(), 80);
}
#[test]
fn palettes_require_three_valid_rgb_colors() {
    let option = w::AttributeOption {
        meta: [
            (
                "colors.center_color".into(),
                "url(https://example.com)".into(),
            ),
            ("colors.edge_color".into(), "#123456".into()),
            ("colors.pattern_color".into(), "#223344".into()),
        ]
        .into(),
        ..Default::default()
    };
    assert!(market_gifts_hub::palette(&option).is_none());
    assert!(market_gifts_hub::palette(&w::AttributeOption::default()).is_none());
}
