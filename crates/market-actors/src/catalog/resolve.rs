//! Resolve model lookups within the caller's pinned market view.
use market_gifts::valuation::{ModelIdentity, ModelLookup, UnpricedReason};
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use std::collections::BTreeMap;
use tonic::Status;

use market_gifts_hub::{CollectionChoices, find_collection, find_model, model_identity};
pub struct ResolvedModel {
    pub identity: Option<ModelIdentity>,
    pub collection: Option<market_gifts::CollectionId>,
    pub collection_label: String,
    pub model_label: String,
    pub unresolved: Option<UnpricedReason>,
}

pub(crate) fn resolve(
    reader: &ReplicaReader,
    lookups: Vec<ModelLookup>,
    cancelled: &tokio_util::sync::CancellationToken,
) -> Result<Vec<ResolvedModel>, Status> {
    let items =
        super::projection::entities_cancellable(reader, w::EntityKind::ItemType, None, cancelled)?;
    let mut options = BTreeMap::new();
    let mut result = Vec::with_capacity(lookups.len());
    for lookup in lookups {
        if cancelled.is_cancelled() {
            return Err(Status::cancelled("Market read cancelled"));
        }
        let (collection, model) = match &lookup {
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
        if !options.contains_key(&item.id) {
            let rules =
                CollectionChoices::new(item).map_err(|e| Status::data_loss(e.to_string()))?;
            let ids = rules.model_option_ids();
            let values = super::projection::trait_options(
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
