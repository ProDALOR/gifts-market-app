use super::TELEGRAM_NAMESPACE;
use market_gifts::{ModelIdentity, ModelLookup, ModelLookupError};
use market_hub_protocol::wire as w;
fn by_name(input: &str, id: &str, name: &str) -> bool {
    id == input || name.eq_ignore_ascii_case(input)
}
/// Numeric transport aliases, interpreted only at the hub adapter boundary.
pub fn telegram_alias_ids(aliases: &[w::ExternalId]) -> impl Iterator<Item = i64> + '_ {
    aliases
        .iter()
        .filter(|a| a.namespace == TELEGRAM_NAMESPACE)
        .filter_map(|a| a.id.parse().ok())
}
pub fn has_telegram_alias(input: i64, aliases: &[w::ExternalId]) -> bool {
    aliases
        .iter()
        .any(|alias| alias.namespace == TELEGRAM_NAMESPACE && alias.id.parse::<i64>() == Ok(input))
}
pub(crate) fn collection_matches(lookup: &ModelLookup, item: &w::ItemType) -> bool {
    match lookup {
        ModelLookup::Name { collection, .. } => by_name(collection, &item.id, &item.name),
        ModelLookup::Catalog { collection, .. } | ModelLookup::Collection { collection } => {
            collection.as_str() == item.id
        }
        ModelLookup::Telegram { gift_id, .. } => has_telegram_alias(gift_id.get(), &item.aliases),
    }
}
pub(crate) fn model_matches(lookup: &ModelLookup, option: &w::AttributeOption) -> bool {
    match lookup {
        ModelLookup::Collection { .. } => false,
        ModelLookup::Name { model, .. } => by_name(model, &option.id, &option.name),
        ModelLookup::Catalog { model, .. } => model.as_str() == option.id,
        ModelLookup::Telegram { document_id, .. } => {
            has_telegram_alias(document_id.get(), &option.aliases)
        }
    }
}

fn unique<T>(
    mut matches: impl Iterator<Item = T>,
    missing: ModelLookupError,
    ambiguous: ModelLookupError,
) -> std::result::Result<T, ModelLookupError> {
    let first = matches.next().ok_or(missing)?;
    if matches.next().is_some() {
        Err(ambiguous)
    } else {
        Ok(first)
    }
}
pub fn find_collection<'a>(
    lookup: &ModelLookup,
    items: impl Iterator<Item = &'a w::ItemType>,
) -> std::result::Result<&'a w::ItemType, ModelLookupError> {
    unique(
        items.filter(|item| super::is_gift(item) && collection_matches(lookup, item)),
        ModelLookupError::UnknownCollection,
        ModelLookupError::AmbiguousCollection,
    )
}
/// The caller supplies the collection's allowed model options, or all options for an unrestricted rule.
pub fn find_model<'a>(
    lookup: &ModelLookup,
    options: impl Iterator<Item = &'a w::AttributeOption>,
) -> std::result::Result<&'a w::AttributeOption, ModelLookupError> {
    unique(
        options
            .filter(|option| option.attribute_id == super::MODEL && model_matches(lookup, option)),
        ModelLookupError::UnknownModel,
        ModelLookupError::AmbiguousModel,
    )
}
pub fn model_identity(
    item: &w::ItemType,
    option: &w::AttributeOption,
) -> super::Result<ModelIdentity> {
    Ok(ModelIdentity {
        collection_id: item.id.clone().try_into().map_err(super::invalid)?,
        model_id: option.id.clone().try_into().map_err(super::invalid)?,
    })
}
