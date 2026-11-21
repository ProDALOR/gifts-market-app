use super::{
    BACKDROP, COLLECTION_SLUG_NAMESPACE, MODEL, PATTERN, Result, TELEGRAM_NAMESPACE, invalid,
};
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::collections::BTreeSet;
pub fn document(option: &w::AttributeOption) -> Option<TelegramDocumentId> {
    option
        .aliases
        .iter()
        .filter(|alias| alias.namespace == TELEGRAM_NAMESPACE)
        .find_map(|alias| alias.id.clone().try_into().ok())
}
pub fn is_gift(item: &w::ItemType) -> bool {
    item.attributes.contains_key(MODEL)
        || item.aliases.iter().any(|alias| {
            (alias.namespace == TELEGRAM_NAMESPACE
                && alias.id.parse::<i64>().is_ok_and(|id| id > 0))
                || alias.namespace == COLLECTION_SLUG_NAMESPACE
        })
}
/// Explicit identity only: display names are mutable and must not generate lookup keys.
pub fn collection_slug(item: &w::ItemType) -> Result<Option<CollectionSlug>> {
    let mut slug = None;
    for alias in item
        .aliases
        .iter()
        .filter(|a| a.namespace == COLLECTION_SLUG_NAMESPACE)
    {
        let next = CollectionSlug::new(alias.id.clone()).map_err(invalid)?;
        if slug.as_ref().is_some_and(|current| current != &next) {
            return Err(invalid("conflicting canonical collection slugs"));
        }
        if slug.is_none() {
            slug = Some(next);
        }
    }
    Ok(slug)
}
pub fn collection(item: w::ItemType) -> Result<GiftCollection> {
    let slug = collection_slug(&item)?;
    Ok(GiftCollection {
        slug,
        id: item.id.try_into().map_err(invalid)?,
        name: item.name,
        preview: None,
    })
}
/// Missing traits allow no choices; a rule without a list allows any choice.
enum AllowedValues<T> {
    Any,
    Only(BTreeSet<T>),
}
impl<T> AllowedValues<T> {
    fn ids(&self) -> Option<&BTreeSet<T>> {
        match self {
            Self::Any => None,
            Self::Only(ids) => Some(ids),
        }
    }
}
impl<T: Ord + std::borrow::Borrow<str>> AllowedValues<T> {
    fn accepts(&self, id: &str) -> bool {
        self.ids().is_none_or(|ids| ids.contains(id))
    }
}
fn allowed<T: Ord + TryFrom<String>>(item: &w::ItemType, key: &str) -> Result<AllowedValues<T>>
where
    T::Error: std::fmt::Display,
{
    match item.attributes.get(key) {
        None => Ok(AllowedValues::Only(BTreeSet::new())),
        Some(rule) => match &rule.allowed_values {
            None => Ok(AllowedValues::Any),
            Some(values) => Ok(AllowedValues::Only(
                values
                    .ids
                    .iter()
                    .cloned()
                    .map(|id| id.try_into().map_err(invalid))
                    .collect::<Result<_>>()?,
            )),
        },
    }
}
pub fn palette(option: &w::AttributeOption) -> Option<BackdropPalette> {
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
pub fn preview_model(item: &w::ItemType) -> Option<&str> {
    item.attributes
        .get(MODEL)?
        .allowed_values
        .as_ref()?
        .ids
        .first()
        .map(String::as_str)
}
pub fn media(option: &w::AttributeOption) -> Option<GiftMedia> {
    document(option)?;
    let target = match option.attribute_id.as_str() {
        MODEL => MediaTarget::Model(option.id.clone().try_into().ok()?),
        PATTERN => MediaTarget::Pattern(option.id.clone().try_into().ok()?),
        _ => return None,
    };
    Some(GiftMedia { target })
}
pub fn model_media(option: &w::AttributeOption) -> Option<GiftMedia> {
    (option.attribute_id == MODEL)
        .then(|| media(option))
        .flatten()
}
pub fn media_document(
    option: &w::AttributeOption,
    target: &MediaTarget,
) -> Option<TelegramDocumentId> {
    let (attribute, id) = match target {
        MediaTarget::Model(id) => (MODEL, id.as_str()),
        MediaTarget::Pattern(id) => (PATTERN, id.as_str()),
    };
    (option.attribute_id == attribute && option.id == id)
        .then(|| document(option))
        .flatten()
}
pub struct CollectionChoices {
    models: AllowedValues<ModelId>,
    backdrops: AllowedValues<BackdropId>,
    patterns: AllowedValues<PatternId>,
}
impl CollectionChoices {
    pub fn new(item: &w::ItemType) -> Result<Self> {
        Ok(Self {
            models: allowed(item, MODEL)?,
            backdrops: allowed(item, BACKDROP)?,
            patterns: allowed(item, PATTERN)?,
        })
    }
    /// None means unrestricted; an empty list means this collection has no models.
    pub fn model_option_ids(&self) -> Option<Vec<String>> {
        self.models
            .ids()
            .map(|ids| ids.iter().map(ToString::to_string).collect())
    }
    pub fn trait_option_ids(&self, attribute: &str) -> Option<Vec<String>> {
        match attribute {
            MODEL => self
                .models
                .ids()
                .map(|ids| ids.iter().map(ToString::to_string).collect()),
            BACKDROP => self
                .backdrops
                .ids()
                .map(|ids| ids.iter().map(ToString::to_string).collect()),
            PATTERN => self
                .patterns
                .ids()
                .map(|ids| ids.iter().map(ToString::to_string).collect()),
            _ => Some(Vec::new()),
        }
    }
    pub fn unrestricted_attributes(&self) -> Vec<String> {
        [
            (MODEL, self.models.ids().is_none()),
            (BACKDROP, self.backdrops.ids().is_none()),
            (PATTERN, self.patterns.ids().is_none()),
        ]
        .into_iter()
        .filter(|(_, unrestricted)| *unrestricted)
        .map(|(attribute, _)| attribute.to_owned())
        .collect()
    }
    pub fn option_ids(&self) -> Vec<String> {
        self.models
            .ids()
            .into_iter()
            .flatten()
            .map(AsRef::as_ref)
            .chain(
                self.backdrops
                    .ids()
                    .into_iter()
                    .flatten()
                    .map(AsRef::as_ref),
            )
            .chain(self.patterns.ids().into_iter().flatten().map(AsRef::as_ref))
            .collect::<BTreeSet<&str>>()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }
    pub fn accepts(&self, option: &w::AttributeOption) -> bool {
        match option.attribute_id.as_str() {
            MODEL => self.models.accepts(&option.id),
            BACKDROP => self.backdrops.accepts(&option.id),
            PATTERN => self.patterns.accepts(&option.id),
            _ => false,
        }
    }
}
pub enum Choice {
    Model(GiftModel),
    Backdrop(GiftBackdrop),
    Pattern(GiftPattern),
}
/// Optional descriptive metadata must not prevent the collection from loading.
fn rarity(option: &w::AttributeOption) -> Option<GiftRarity> {
    if let Some(value) = option.meta.get("rarity.permille") {
        return Some(GiftRarity::Percentage {
            permille: value.parse::<u16>().ok()?.try_into().ok()?,
        });
    }
    match option.meta.get("rarity.label")?.as_str() {
        "uncommon" => Some(GiftRarity::Uncommon),
        "rare" => Some(GiftRarity::Rare),
        "epic" => Some(GiftRarity::Epic),
        "legendary" => Some(GiftRarity::Legendary),
        _ => None,
    }
}
pub fn choice(option: w::AttributeOption) -> Result<Option<Choice>> {
    Ok(match option.attribute_id.as_str() {
        MODEL => Some(Choice::Model(GiftModel {
            id: option.id.clone().try_into().map_err(invalid)?,
            name: option.name.clone(),
            media: media(&option),
            rarity: rarity(&option),
        })),
        BACKDROP => Some(Choice::Backdrop(GiftBackdrop {
            id: option.id.clone().try_into().map_err(invalid)?,
            name: option.name.clone(),
            palette: palette(&option),
            rarity: rarity(&option),
        })),
        PATTERN => Some(Choice::Pattern(GiftPattern {
            id: option.id.clone().try_into().map_err(invalid)?,
            name: option.name.clone(),
            media: media(&option),
            rarity: rarity(&option),
        })),
        _ => None,
    })
}
