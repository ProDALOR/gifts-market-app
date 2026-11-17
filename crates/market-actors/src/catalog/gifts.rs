//! Gift catalog projection over one pinned replica view.
use crate::protocol::{BACKDROP, MODEL, PATTERN};
use market_gifts::*;
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use std::collections::BTreeSet;
use tonic::Status;
fn invalid(value: impl std::fmt::Display) -> Status {
    Status::data_loss(value.to_string())
}
pub(super) fn read(
    reader: &ReplicaReader,
    kind: w::EntityKind,
    ids: Option<&[String]>,
) -> Result<(Vec<w::entity::Entity>, String), Status> {
    if let Some(ids) = ids.filter(|ids| ids.len() > 500) {
        if ids.len() > 50_000 {
            return Err(Status::resource_exhausted(
                "gift catalog exceeds response budget",
            ));
        }
        let mut rows = Vec::new();
        let mut cursor = String::new();
        for chunk in ids.chunks(500) {
            let (part, checkpoint) = read(reader, kind, Some(chunk))?;
            rows.extend(part);
            cursor = checkpoint;
        }
        return Ok((rows, cursor));
    }
    if ids.is_some_and(<[String]>::is_empty) {
        return Ok((vec![], String::new()));
    }
    let mut request = w::ReadRequest {
        selection: Some(w::Selection {
            kinds: vec![kind as i32],
            keys: ids
                .unwrap_or_default()
                .iter()
                .map(|id| w::Key {
                    kind: kind as i32,
                    id: id.clone(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
        mode: w::ReadMode::Live as i32,
        limit: 500,
        ..Default::default()
    };
    let mut rows = Vec::new();
    loop {
        let result = reader.read(request.clone())?;
        rows.extend(
            result
                .page
                .entities
                .into_iter()
                .filter_map(|row| row.entity),
        );
        if rows.len() > 50_000 {
            return Err(Status::resource_exhausted(
                "gift catalog exceeds response budget",
            ));
        }
        if result.page.next_page_token.is_empty() {
            return Ok((rows, result.page.cursor));
        }
        request.page_token = result.page.next_page_token;
    }
}
pub(super) fn is_gift(item: &w::ItemType) -> bool {
    item.attributes.contains_key(MODEL)
        || item
            .aliases
            .iter()
            .any(|alias| alias.namespace == "tg" && alias.id.parse::<i64>().is_ok_and(|id| id > 0))
}
fn allowed<T: TryFrom<String>>(item: &w::ItemType, key: &str) -> Result<Option<Vec<T>>, Status>
where
    T::Error: std::fmt::Display,
{
    match item.attributes.get(key) {
        None => Ok(Some(vec![])),
        Some(rule) => rule
            .allowed_values
            .as_ref()
            .map(|values| {
                values
                    .ids
                    .iter()
                    .cloned()
                    .map(|id| id.try_into().map_err(invalid))
                    .collect()
            })
            .transpose(),
    }
}
fn collection(item: w::ItemType) -> Result<GiftCollection, Status> {
    Ok(GiftCollection {
        id: item.id.clone().try_into().map_err(invalid)?,
        name: item.name.clone(),
        preview: None,
    })
}
pub(super) fn validate_collection(reader: &ReplicaReader, id: &str) -> Result<(), Status> {
    let (items, _) = read(reader, w::EntityKind::ItemType, Some(&[id.to_owned()]))?;
    if items
        .iter()
        .any(|item| matches!(item, w::entity::Entity::ItemType(item) if is_gift(item)))
    {
        Ok(())
    } else {
        Err(Status::invalid_argument("unknown gift collection"))
    }
}
pub(super) fn bootstrap(reader: ReplicaReader) -> Result<GiftCatalog, Status> {
    let reader = reader.snapshot()?;
    let (items, cursor) = read(&reader, w::EntityKind::ItemType, None)?;
    let items: Vec<_> = items
        .into_iter()
        .filter_map(|row| match row {
            w::entity::Entity::ItemType(item) if is_gift(&item) => Some(item),
            _ => None,
        })
        .collect();
    let preview_by_collection: std::collections::BTreeMap<_, _> = items
        .iter()
        .filter_map(|item| {
            item.attributes
                .get(MODEL)?
                .allowed_values
                .as_ref()?
                .ids
                .first()
                .map(|id| (item.id.clone(), id.clone()))
        })
        .collect();
    let mut collections: Vec<_> = items
        .into_iter()
        .map(collection)
        .collect::<Result<_, _>>()?;
    let preview_ids: Vec<_> = preview_by_collection
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let previews: std::collections::BTreeMap<_, _> =
        read(&reader, w::EntityKind::AttributeOption, Some(&preview_ids))?
            .0
            .into_iter()
            .filter_map(|row| match row {
                w::entity::Entity::AttributeOption(option) if option.attribute_id == MODEL => {
                    media(&option, true).map(|media| (option.id, media))
                }
                _ => None,
            })
            .collect();
    for collection in &mut collections {
        collection.preview = preview_by_collection
            .get(collection.id.as_str())
            .and_then(|id| previews.get(id))
            .cloned();
    }
    let venues = read(&reader, w::EntityKind::Venue, None)?
        .0
        .into_iter()
        .filter_map(|row| {
            if let w::entity::Entity::Venue(v) = row {
                Some(v)
            } else {
                None
            }
        })
        .map(|v| {
            Ok(Venue {
                id: v.id.try_into().map_err(invalid)?,
                name: v.name,
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;
    let currencies = read(&reader, w::EntityKind::Currency, None)?
        .0
        .into_iter()
        .filter_map(|row| {
            if let w::entity::Entity::Currency(v) = row {
                Some(v)
            } else {
                None
            }
        })
        .map(|v| {
            Ok(Currency {
                id: v.id.try_into().map_err(invalid)?,
                name: v.name,
                decimals: v.decimals,
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;
    let mut books = Vec::new();
    let mut after = None;
    loop {
        let page = reader.books(after, 500)?;
        for book in page.books {
            if collections
                .iter()
                .any(|c| c.id.as_str() == book.item_type_id)
            {
                books.push(crate::protocol::book(book).map_err(invalid)?);
            }
        }
        if books.len() > 50_000 {
            return Err(Status::resource_exhausted(
                "gift directory exceeds response budget",
            ));
        }
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    Ok(GiftCatalog {
        collections: collections.into(),
        venues: venues.into(),
        currencies: currencies.into(),
        books: books.into(),
        cursor,
        sync: crate::protocol::sync(reader.status()).map_err(invalid)?,
    })
}
fn document(option: &w::AttributeOption) -> Option<TelegramDocumentId> {
    option
        .aliases
        .iter()
        .filter(|alias| alias.namespace == "tg")
        .find_map(|alias| alias.id.clone().try_into().ok())
}
const MEDIA_COMPONENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'.')
    .remove(b'-')
    .remove(b'_')
    .remove(b'~');
fn media(option: &w::AttributeOption, model: bool) -> Option<GiftMedia> {
    document(option)?;
    // Catalog IDs are encoded as URL components by the transport-safe byte encoder.
    let id = percent_encoding::utf8_percent_encode(&option.id, MEDIA_COMPONENT).to_string();
    let thumbnail_url = format!("/media/{}/{id}", if model { "models" } else { "patterns" });
    Some(GiftMedia {
        animation_url: model.then(|| format!("{thumbnail_url}/animation")),
        thumbnail_url,
    })
}
fn palette(option: &w::AttributeOption) -> Option<BackdropPalette> {
    let color = |key: &str| {
        option
            .meta
            .get(key)
            .cloned()
            .and_then(|value| value.try_into().ok())
    };
    Some(BackdropPalette {
        center: color("colors.center_color")?,
        edge: color("colors.edge_color")?,
        pattern: color("colors.pattern_color")?,
    })
}
pub(super) fn details(
    reader: ReplicaReader,
    id: CollectionId,
) -> Result<GiftCollectionCatalog, Status> {
    let reader = reader.snapshot()?;
    let (items, cursor) = read(&reader, w::EntityKind::ItemType, Some(&[id.to_string()]))?;
    let item = items
        .into_iter()
        .find_map(|row| {
            if let w::entity::Entity::ItemType(item) = row {
                is_gift(&item).then_some(item)
            } else {
                None
            }
        })
        .ok_or_else(|| Status::not_found("gift collection not found"))?;
    let ids = allowed::<ModelId>(&item, MODEL)?
        .map(|ids| ids.into_iter().map(String::from).collect::<BTreeSet<_>>());
    let backdrops = allowed::<BackdropId>(&item, BACKDROP)?
        .map(|ids| ids.into_iter().map(String::from).collect::<BTreeSet<_>>());
    let patterns = allowed::<PatternId>(&item, PATTERN)?
        .map(|ids| ids.into_iter().map(String::from).collect::<BTreeSet<_>>());
    let collection = collection(item)?;
    let unrestricted = ids.is_none() || backdrops.is_none() || patterns.is_none();
    let requested: Vec<_> = ids
        .iter()
        .chain(backdrops.iter())
        .chain(patterns.iter())
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let options = read(
        &reader,
        w::EntityKind::AttributeOption,
        if unrestricted { None } else { Some(&requested) },
    )?
    .0;
    let mut result = GiftCollectionCatalog {
        collection,
        models: Default::default(),
        backdrops: Default::default(),
        patterns: Default::default(),
        cursor,
        sync: crate::protocol::sync(reader.status()).map_err(invalid)?,
    };
    for row in options {
        if let w::entity::Entity::AttributeOption(option) = row {
            match option.attribute_id.as_str() {
                MODEL if ids.as_ref().is_none_or(|ids| ids.contains(&option.id)) => {
                    std::sync::Arc::make_mut(&mut result.models).push(GiftModel {
                        id: option.id.clone().try_into().map_err(invalid)?,
                        name: option.name.clone(),
                        media: media(&option, true),
                    })
                }
                BACKDROP
                    if backdrops
                        .as_ref()
                        .is_none_or(|ids| ids.contains(&option.id)) =>
                {
                    std::sync::Arc::make_mut(&mut result.backdrops).push(GiftBackdrop {
                        id: option.id.clone().try_into().map_err(invalid)?,
                        name: option.name.clone(),
                        palette: palette(&option),
                    })
                }
                PATTERN if patterns.as_ref().is_none_or(|ids| ids.contains(&option.id)) => {
                    std::sync::Arc::make_mut(&mut result.patterns).push(GiftPattern {
                        id: option.id.clone().try_into().map_err(invalid)?,
                        name: option.name.clone(),
                        media: media(&option, false),
                    })
                }
                _ => {}
            }
        }
    }
    result.collection.preview = result.models.iter().find_map(|model| model.media.clone());
    Ok(result)
}
pub(super) fn media_document(
    reader: ReplicaReader,
    target: MediaTarget,
) -> Result<TelegramDocumentId, Status> {
    let (id, kind) = match target {
        MediaTarget::Model(id) => (id.to_string(), MODEL),
        MediaTarget::Pattern(id) => (id.to_string(), PATTERN),
    };
    let reader = reader.snapshot()?;
    read(&reader, w::EntityKind::AttributeOption, Some(&[id]))?
        .0
        .into_iter()
        .find_map(|row| match row {
            w::entity::Entity::AttributeOption(option) if option.attribute_id == kind => {
                document(&option)
            }
            _ => None,
        })
        .ok_or_else(|| Status::not_found("gift media not found"))
}
pub(super) fn model_documents(reader: ReplicaReader) -> Result<Vec<TelegramDocumentId>, Status> {
    let reader = reader.snapshot()?;
    let mut documents = BTreeSet::new();
    let mut token = String::new();
    for _ in 0..250 {
        let result = reader.read(w::ReadRequest {
            selection: Some(w::Selection {
                kinds: vec![w::EntityKind::AttributeOption as i32],
                ..Default::default()
            }),
            limit: 200,
            page_token: token,
            mode: w::ReadMode::Live as i32,
            ..Default::default()
        })?;
        for row in result.page.entities {
            if let Some(w::entity::Entity::AttributeOption(option)) = row.entity
                && option.attribute_id == MODEL
                && let Some(document) = document(&option)
            {
                documents.insert(document);
                if documents.len() == 64 {
                    return Ok(documents.into_iter().collect());
                }
            }
        }
        token = result.page.next_page_token;
        if token.is_empty() {
            break;
        }
    }
    Ok(documents.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let catalog = bootstrap(writer.reader()).unwrap();
        assert_eq!(catalog.collections.len(), 1);
        let preview = catalog.collections[0].preview.as_ref().unwrap();
        assert_eq!(preview.thumbnail_url, "/media/models/model.0");
        assert_eq!(
            preview.animation_url.as_deref(),
            Some("/media/models/model.0/animation")
        );
        assert_eq!(catalog.cursor, "catalog-checkpoint");
        let result = details(writer.reader(), "collection".into()).unwrap();
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
                .thumbnail_url,
            "/media/models/model.0"
        );
        assert_eq!(
            media_document(writer.reader(), MediaTarget::Pattern("pattern".into()))
                .unwrap()
                .get(),
            555
        );
        assert!(media_document(writer.reader(), MediaTarget::Model("pattern".into())).is_err());
        assert!(details(writer.reader(), "not-a-gift".into()).is_err());
        assert_eq!(model_documents(writer.reader()).unwrap().len(), 64);
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
        assert!(palette(&option).is_none());
        assert!(palette(&w::AttributeOption::default()).is_none());
    }
}
