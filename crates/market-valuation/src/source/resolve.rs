//! Resolve model lookups within the caller's pinned market view.
use market_gifts::ModelLookupError;
use market_gifts::{ModelIdentity, ModelLookup};
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use std::collections::{BTreeMap, BTreeSet};
use tonic::Status;

use market_gifts_hub::{CollectionChoices, find_collection, find_model, model_identity};
pub struct ResolvedModel {
    pub identity: Option<ModelIdentity>,
    pub collection: Option<market_gifts::CollectionId>,
    pub collection_label: String,
    pub model_label: String,
    pub unresolved: Option<ModelLookupError>,
}

pub(crate) fn resolve(
    reader: &ReplicaReader,
    lookups: Vec<ModelLookup>,
    cancelled: &tokio_util::sync::CancellationToken,
) -> Result<Vec<ResolvedModel>, Status> {
    if lookups.is_empty() {
        return Ok(vec![]);
    }
    let needs_directory = lookups.iter().any(|lookup| {
        matches!(
            lookup,
            ModelLookup::Name { .. } | ModelLookup::Telegram { .. }
        )
    });
    let ids = lookups
        .iter()
        .filter_map(|lookup| match lookup {
            ModelLookup::Catalog { collection, .. } | ModelLookup::Collection { collection } => {
                Some(collection.to_string())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut requested_models: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for lookup in &lookups {
        if let ModelLookup::Catalog { collection, model } = lookup {
            requested_models
                .entry(collection.to_string())
                .or_default()
                .insert(model.to_string());
        }
    }
    let items = market_replica::catalog_read::entities_cancellable(
        reader,
        w::EntityKind::ItemType,
        (!needs_directory).then_some(ids.as_slice()),
        cancelled,
    )?;
    let mut options = BTreeMap::new();
    let mut result = Vec::with_capacity(lookups.len());
    for lookup in lookups {
        if cancelled.is_cancelled() {
            return Err(Status::cancelled("Market read cancelled"));
        }
        let (collection, model) = match &lookup {
            ModelLookup::Collection { collection } => (collection.to_string(), String::new()),
            ModelLookup::Name { collection, model } => (collection.clone(), model.clone()),
            ModelLookup::Catalog { collection, model } => {
                (collection.to_string(), model.to_string())
            }
            ModelLookup::Telegram {
                gift_id,
                document_id,
            } => (gift_id.get().to_string(), document_id.get().to_string()),
        };
        let mut row = ResolvedModel {
            identity: None,
            collection: None,
            collection_label: collection,
            model_label: model,
            unresolved: None,
        };
        let item = match find_collection(
            &lookup,
            items.iter().filter_map(|item| match item {
                w::entity::Entity::ItemType(item) => Some(item),
                _ => None,
            }),
        ) {
            Ok(item) => item,
            Err(reason) => {
                row.unresolved = Some(reason);
                result.push(row);
                continue;
            }
        };
        row.collection_label = item.name.clone();
        if matches!(lookup, ModelLookup::Collection { .. }) {
            row.collection = Some(
                item.id
                    .clone()
                    .try_into()
                    .map_err(|e| Status::data_loss(format!("{e}")))?,
            );
            result.push(row);
            continue;
        }

        if !options.contains_key(&item.id) {
            let rules =
                CollectionChoices::new(item).map_err(|e| Status::data_loss(e.to_string()))?;
            let ids = if needs_directory {
                rules.model_option_ids()
            } else {
                let allowed = rules.model_option_ids();
                Some(
                    requested_models
                        .get(&item.id)
                        .into_iter()
                        .flatten()
                        .filter(|id| allowed.as_ref().is_none_or(|ids| ids.contains(id)))
                        .cloned()
                        .collect(),
                )
            };
            let values = market_replica::catalog_read::trait_options(
                reader,
                market_gifts_hub::MODEL,
                ids.as_deref(),
                cancelled,
            )?;
            options.insert(item.id.clone(), values);
        }
        let option = match find_model(&lookup, options[&item.id].iter()) {
            Ok(option) => option,
            Err(reason) => {
                row.unresolved = Some(reason);
                row.collection = Some(
                    item.id
                        .clone()
                        .try_into()
                        .map_err(|e| Status::data_loss(format!("{e}")))?,
                );
                result.push(row);
                continue;
            }
        };
        row.model_label = option.name.clone();
        row.identity =
            Some(model_identity(item, option).map_err(|e| Status::data_loss(e.to_string()))?);
        row.collection = Some(
            item.id
                .clone()
                .try_into()
                .map_err(|e| Status::data_loss(format!("{e}")))?,
        );
        result.push(row);
    }
    Ok(result)
}
