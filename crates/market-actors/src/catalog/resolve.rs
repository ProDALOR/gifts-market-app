use crate::valuation::{ModelKey, ModelPosition, ModelValue, ResolvedPosition, UnpricedReason};
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use std::collections::BTreeMap;
use tonic::Status;

fn read_all(
    reader: &ReplicaReader,
    kind: w::EntityKind,
    ids: &[String],
) -> Result<Vec<w::entity::Entity>, Status> {
    let mut request = w::ReadRequest {
        selection: Some(w::Selection {
            kinds: vec![kind as i32],
            keys: ids
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
    let mut values = Vec::new();
    loop {
        let page = reader.read(request.clone())?.page;
        values.extend(page.entities.into_iter().filter_map(|entity| entity.entity));
        if values.len() > 50_000 {
            return Err(Status::resource_exhausted(
                "catalog resolution limit exceeded",
            ));
        }
        if page.next_page_token.is_empty() {
            return Ok(values);
        }
        request.page_token = page.next_page_token;
    }
}
fn matches(
    namespace: &Option<String>,
    input: &str,
    id: &str,
    name: &str,
    aliases: &[w::ExternalId],
) -> bool {
    match namespace {
        Some(namespace) => aliases
            .iter()
            .any(|alias| &alias.namespace == namespace && alias.id == input),
        None => id == input || name.eq_ignore_ascii_case(input),
    }
}
pub(super) fn resolve(
    reader: ReplicaReader,
    positions: Vec<ModelPosition>,
) -> Result<Vec<ResolvedPosition>, Status> {
    let reader = reader.snapshot()?;
    let items = read_all(&reader, w::EntityKind::ItemType, &[])?;
    let mut options = BTreeMap::<(String, String), Vec<w::AttributeOption>>::new();
    let mut result = Vec::with_capacity(positions.len());
    for position in positions {
        let lookup = position.model;
        let mut row = ModelValue {
            collection: lookup.collection.clone(),
            model: lookup.model.clone(),
            quantity: position.quantity,
            floor: None,
            value: None,
            venue_id: None,
            unpriced: None,
        };
        let matched: Vec<_> = items
            .iter()
            .filter_map(|item| {
                if let w::entity::Entity::ItemType(item) = item {
                    Some(item)
                } else {
                    None
                }
            })
            .filter(|item| {
                matches(
                    &lookup.namespace,
                    &lookup.collection,
                    &item.id,
                    &item.name,
                    &item.aliases,
                )
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
            result.push(ResolvedPosition { key: None, row });
            continue;
        };
        row.collection = item.name.clone();
        let cache_key = (item.id.clone(), lookup.attribute_id.clone());
        if !options.contains_key(&cache_key) {
            let mut values = Vec::new();
            if let Some(rule) = item.attributes.get(&lookup.attribute_id) {
                let entities = if let Some(ids) = &rule.allowed_values {
                    let mut entities = Vec::new();
                    for chunk in ids.ids.chunks(500) {
                        entities.extend(read_all(&reader, w::EntityKind::AttributeOption, chunk)?);
                    }
                    entities
                } else {
                    read_all(&reader, w::EntityKind::AttributeOption, &[])?
                };
                values.extend(entities.into_iter().filter_map(|entity| {
                    if let w::entity::Entity::AttributeOption(option) = entity {
                        Some(option)
                    } else {
                        None
                    }
                }));
            }
            options.insert(cache_key.clone(), values);
        }
        let matched: Vec<_> = options[&cache_key]
            .iter()
            .filter(|option| {
                option.attribute_id == lookup.attribute_id
                    && matches(
                        &lookup.namespace,
                        &lookup.model,
                        &option.id,
                        &option.name,
                        &option.aliases,
                    )
            })
            .collect();
        let option = if let [option] = matched.as_slice() {
            *option
        } else {
            row.unpriced = Some(if matched.is_empty() {
                UnpricedReason::UnknownModel
            } else {
                UnpricedReason::AmbiguousModel
            });
            result.push(ResolvedPosition { key: None, row });
            continue;
        };
        row.model = option.name.clone();
        result.push(ResolvedPosition {
            key: Some(ModelKey {
                collection: item.id.clone(),
                attribute: lookup.attribute_id,
                option: option.id.clone(),
            }),
            row,
        });
    }
    Ok(result)
}
