//! Models, backdrops and patterns available within a collection.
use super::{
    collections::{collection, is_gift},
    read::{entities, invalid},
};
use market_gifts::*;
use market_hub_protocol::wire as w;
use market_replica::{MetadataDependencies, ReplicaReader};
use tonic::Status;
pub(crate) struct ChoiceProjection {
    pub catalog: GiftCollectionCatalog,
    pub dependencies: MetadataDependencies,
    pub revision: std::sync::OnceLock<Result<String, Status>>,
}
impl ChoiceProjection {
    pub fn weight(&self) -> usize {
        let catalog = &self.catalog;
        self.dependencies
            .entity_count()
            .max(catalog.models.len() + catalog.backdrops.len() + catalog.patterns.len())
    }
}
pub(crate) fn choices(
    reader: &ReplicaReader,
    id: CollectionId,
) -> Result<ChoiceProjection, Status> {
    let items = entities(reader, w::EntityKind::ItemType, Some(&[id.to_string()]))?;
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
    let rules = market_gifts_hub::CollectionChoices::new(&item).map_err(invalid)?;
    let collection = collection(item).map_err(invalid)?;
    let option_attributes = rules.unrestricted_attributes();
    let unrestricted = !option_attributes.is_empty();
    let requested = rules.option_ids();
    let mut keys = vec![w::Key {
        kind: w::EntityKind::ItemType as i32,
        id: id.to_string(),
        ..Default::default()
    }];
    keys.extend(requested.iter().map(|id| w::Key {
        kind: w::EntityKind::AttributeOption as i32,
        id: id.clone(),
        ..Default::default()
    }));
    let dependencies = reader.metadata_dependencies(keys, option_attributes)?;
    let options = entities(
        reader,
        w::EntityKind::AttributeOption,
        if unrestricted { None } else { Some(&requested) },
    )?;
    let mut result = GiftCollectionCatalog {
        collection,
        models: Default::default(),
        backdrops: Default::default(),
        patterns: Default::default(),
    };
    for row in options {
        if let w::entity::Entity::AttributeOption(option) = row {
            if !rules.accepts(&option) {
                continue;
            }
            use market_gifts_hub::Choice;
            match market_gifts_hub::choice(option).map_err(invalid)? {
                Some(Choice::Model(model)) => {
                    std::sync::Arc::make_mut(&mut result.models).push(model)
                }
                Some(Choice::Backdrop(backdrop)) => {
                    std::sync::Arc::make_mut(&mut result.backdrops).push(backdrop)
                }
                Some(Choice::Pattern(pattern)) => {
                    std::sync::Arc::make_mut(&mut result.patterns).push(pattern)
                }
                None => {}
            }
        }
    }
    result.collection.preview = result.models.iter().find_map(|model| model.media.clone());
    Ok(ChoiceProjection {
        catalog: result,
        dependencies,
        revision: Default::default(),
    })
}
