//! Resolve model lookups within the caller's pinned valuation view.
use market_gifts::valuation::{
    ModelIdentity, ModelLookup, ModelPosition, ModelValue, UnpricedReason,
};
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use std::collections::BTreeMap;
use tonic::Status;

fn by_name(input: &str, id: &str, name: &str) -> bool {
    id == input || name.eq_ignore_ascii_case(input)
}
fn by_telegram(input: i64, aliases: &[w::ExternalId]) -> bool {
    aliases
        .iter()
        .any(|alias| alias.namespace == "tg" && alias.id.parse::<i64>() == Ok(input))
}
fn collection_matches(lookup: &ModelLookup, item: &w::ItemType) -> bool {
    match lookup {
        ModelLookup::Name { collection, .. } => by_name(collection, &item.id, &item.name),
        ModelLookup::Catalog { collection, .. } => collection.as_str() == item.id,
        ModelLookup::Telegram { gift_id, .. } => by_telegram(gift_id.get(), &item.aliases),
    }
}
fn model_matches(lookup: &ModelLookup, option: &w::AttributeOption) -> bool {
    match lookup {
        ModelLookup::Name { model, .. } => by_name(model, &option.id, &option.name),
        ModelLookup::Catalog { model, .. } => model.as_str() == option.id,
        ModelLookup::Telegram { document_id, .. } => {
            by_telegram(document_id.get(), &option.aliases)
        }
    }
}
pub(crate) fn resolve(
    reader: &ReplicaReader,
    positions: Vec<ModelPosition>,
) -> Result<Vec<ModelValue>, Status> {
    let items = super::gifts::read(reader, w::EntityKind::ItemType, None)?.0;
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
        let matched: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                w::entity::Entity::ItemType(item)
                    if super::gifts::is_gift(item) && collection_matches(&position.model, item) =>
                {
                    Some(item)
                }
                _ => None,
            })
            .collect();
        let item = if let [item] = matched.as_slice() {
            *item
        } else {
            row.unpriced = Some(if matched.is_empty() {
                UnpricedReason::UnknownCollection
            } else {
                UnpricedReason::AmbiguousCollection
            });
            result.push(row);
            continue;
        };
        row.collection = item.name.clone();
        if !options.contains_key(&item.id) {
            let values = if let Some(rule) = item.attributes.get(crate::protocol::MODEL) {
                super::gifts::read(
                    reader,
                    w::EntityKind::AttributeOption,
                    rule.allowed_values.as_ref().map(|ids| ids.ids.as_slice()),
                )?
                .0
            } else {
                vec![]
            };
            options.insert(
                item.id.clone(),
                values
                    .into_iter()
                    .filter_map(|value| match value {
                        w::entity::Entity::AttributeOption(option)
                            if option.attribute_id == crate::protocol::MODEL =>
                        {
                            Some(option)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let matched: Vec<_> = options[&item.id]
            .iter()
            .filter(|option| model_matches(&position.model, option))
            .collect();
        let option = if let [option] = matched.as_slice() {
            *option
        } else {
            row.unpriced = Some(if matched.is_empty() {
                UnpricedReason::UnknownModel
            } else {
                UnpricedReason::AmbiguousModel
            });
            result.push(row);
            continue;
        };
        row.model = option.name.clone();
        row.identity = Some(ModelIdentity {
            collection_id: item
                .id
                .clone()
                .try_into()
                .map_err(|e| Status::data_loss(format!("{e}")))?,
            model_id: option
                .id
                .clone()
                .try_into()
                .map_err(|e| Status::data_loss(format!("{e}")))?,
        });
        result.push(row);
    }
    Ok(result)
}
