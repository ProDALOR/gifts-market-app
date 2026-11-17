//! Resolve model lookups within the caller's pinned valuation view.
use market_gifts::valuation::{ModelLookup, ModelPosition, ModelValue};
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use std::collections::BTreeMap;
use tonic::Status;

use market_gifts_hub::{CollectionChoices, find_collection, find_model, model_identity};
pub(crate) fn resolve(
    reader: &ReplicaReader,
    positions: Vec<ModelPosition>,
) -> Result<Vec<ModelValue>, Status> {
    let items = super::projection::entities(reader, w::EntityKind::ItemType, None)?;
    let mut options = BTreeMap::new();
    let mut result = Vec::with_capacity(positions.len());
    for position in positions {
        let (collection, model) = match &position.model {
            ModelLookup::Name { collection, model } => (collection.clone(), model.clone()),
            ModelLookup::Catalog { collection, model } => {
                (collection.to_string(), model.to_string())
            }
            ModelLookup::Telegram {
                gift_id,
                document_id,
            } => (gift_id.get().to_string(), document_id.get().to_string()),
        };
        let mut row = ModelValue {
            identity: None,
            collection,
            model,
            quantity: position.quantity,
            floor: None,
            value: None,
            venue_id: None,
            unpriced: None,
        };
        let item = match find_collection(
            &position.model,
            items.iter().filter_map(|item| match item {
                w::entity::Entity::ItemType(item) => Some(item),
                _ => None,
            }),
        ) {
            Ok(item) => item,
            Err(reason) => {
                row.unpriced = Some(reason);
                result.push(row);
                continue;
            }
        };
        row.collection = item.name.clone();
        if !options.contains_key(&item.id) {
            let rules =
                CollectionChoices::new(item).map_err(|e| Status::data_loss(e.to_string()))?;
            let ids = rules.model_option_ids();
            let values = super::projection::entities(
                reader,
                w::EntityKind::AttributeOption,
                ids.as_deref(),
            )?;
            options.insert(
                item.id.clone(),
                values
                    .into_iter()
                    .filter_map(|value| match value {
                        w::entity::Entity::AttributeOption(option) => Some(option),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let option = match find_model(&position.model, options[&item.id].iter()) {
            Ok(option) => option,
            Err(reason) => {
                row.unpriced = Some(reason);
                result.push(row);
                continue;
            }
        };
        row.model = option.name.clone();
        row.identity =
            Some(model_identity(item, option).map_err(|e| Status::data_loss(e.to_string()))?);
        result.push(row);
    }
    Ok(result)
}
