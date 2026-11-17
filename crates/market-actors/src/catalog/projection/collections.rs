//! Gift collection summaries and validation.
use super::read::{entities, invalid};
use market_gifts::*;
pub(crate) use market_gifts_hub::{collection, is_gift};
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use std::{collections::BTreeSet, sync::Arc};
use tonic::Status;
pub(crate) fn validate_collection(reader: &ReplicaReader, id: &str) -> Result<(), Status> {
    let items = entities(reader, w::EntityKind::ItemType, Some(&[id.to_owned()]))?;
    if items
        .iter()
        .any(|item| matches!(item, w::entity::Entity::ItemType(item) if is_gift(item)))
    {
        Ok(())
    } else {
        Err(Status::invalid_argument("unknown gift collection"))
    }
}
pub(crate) fn collections(reader: &ReplicaReader) -> Result<Arc<Vec<GiftCollection>>, Status> {
    let items = entities(reader, w::EntityKind::ItemType, None)?;
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
            market_gifts_hub::preview_model(item).map(|id| (item.id.clone(), id.to_owned()))
        })
        .collect();
    let mut collections: Vec<_> = items
        .into_iter()
        .map(|item| collection(item).map_err(invalid))
        .collect::<Result<_, _>>()?;
    let preview_ids: Vec<_> = preview_by_collection
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let previews: std::collections::BTreeMap<_, _> =
        entities(reader, w::EntityKind::AttributeOption, Some(&preview_ids))?
            .into_iter()
            .filter_map(|row| match row {
                w::entity::Entity::AttributeOption(option) => {
                    market_gifts_hub::model_media(&option).map(|media| (option.id, media))
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
    Ok(collections.into())
}
