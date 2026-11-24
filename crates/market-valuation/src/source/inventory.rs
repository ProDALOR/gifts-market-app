//! Resolve transport attributes in batches: one catalog traversal and one choice read per collection/kind.
use super::*;
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::collections::{BTreeMap, BTreeSet};
type Aliases = BTreeMap<i64, Option<String>>;
struct Choices {
    collection: CollectionId,
    model: Aliases,
    backdrop: Aliases,
    pattern: Aliases,
}
fn choice(map: &Aliases, id: i64) -> std::result::Result<String, String> {
    map.get(&id)
        .and_then(Option::as_ref)
        .cloned()
        .ok_or_else(|| "Gift trait is missing or ambiguous in the catalog".into())
}
impl ValuationSnapshot {
    pub(crate) fn inventory_gifts(
        &self,
        gifts: &[market_gifts::inventory::InventoryGift],
    ) -> Result<Vec<std::result::Result<CollectibleGift, String>>> {
        self.check_cancelled()?;
        if gifts.is_empty() {
            return Ok(Vec::new());
        }
        let items = market_replica::catalog_read::entities_cancellable(
            &self.reader,
            w::EntityKind::ItemType,
            None,
            &self.cancelled,
        )?;
        let wanted: BTreeSet<_> = gifts.iter().map(|g| g.gift.collection).collect();
        let mut collections: BTreeMap<i64, Option<&w::ItemType>> = BTreeMap::new();
        for row in &items {
            self.check_cancelled()?;
            if let w::entity::Entity::ItemType(item) = row
                && market_gifts_hub::is_gift(item)
            {
                for id in
                    market_gifts_hub::telegram_alias_ids(&item.aliases).collect::<BTreeSet<_>>()
                {
                    if let Ok(key) = TelegramGiftId::new(id)
                        && wanted.contains(&key)
                    {
                        collections
                            .entry(id)
                            .and_modify(|v| *v = None)
                            .or_insert(Some(item));
                    }
                }
            }
        }
        let mut choices = BTreeMap::new();
        for id in wanted {
            self.check_cancelled()?;
            if let Some(Some(item)) = collections.get(&id.get()) {
                let rules = market_gifts_hub::CollectionChoices::new(item)
                    .map_err(|_| Status::unavailable("Invalid collection catalog"))?;
                let mut maps = Vec::new();
                for kind in [
                    market_gifts_hub::MODEL,
                    market_gifts_hub::BACKDROP,
                    market_gifts_hub::PATTERN,
                ] {
                    let ids = rules.trait_option_ids(kind);
                    let options = market_replica::catalog_read::trait_options(
                        &self.reader,
                        kind,
                        ids.as_deref(),
                        &self.cancelled,
                    )?;
                    let mut map: Aliases = BTreeMap::new();
                    for option in options.iter().filter(|v| rules.accepts(v)) {
                        self.check_cancelled()?;
                        for alias in market_gifts_hub::telegram_alias_ids(&option.aliases) {
                            map.entry(alias)
                                .and_modify(|old| {
                                    if old.as_ref() != Some(&option.id) {
                                        *old = None;
                                    }
                                })
                                .or_insert_with(|| Some(option.id.clone()));
                        }
                    }
                    maps.push(map);
                }
                let pattern = maps.pop().unwrap();
                let backdrop = maps.pop().unwrap();
                let model = maps.pop().unwrap();
                choices.insert(
                    id,
                    Choices {
                        collection: item
                            .id
                            .clone()
                            .try_into()
                            .map_err(|_| Status::unavailable("Invalid collection ID"))?,
                        model,
                        backdrop,
                        pattern,
                    },
                );
            }
        }
        Ok(gifts
            .iter()
            .map(|item| {
                let gift = &item.gift;
                let Some(choices) = choices.get(&gift.collection) else {
                    return Err("Gift collection is missing or ambiguous in the catalog".into());
                };
                let convert = || -> std::result::Result<CollectibleGift, String> {
                    Ok(CollectibleGift {
                        id: gift.id.clone(),
                        collection_id: choices.collection.clone(),
                        number: Some(gift.number),
                        traits: GiftTraits {
                            model: Some(
                                choice(&choices.model, gift.model.get())?
                                    .try_into()
                                    .map_err(|_| "Invalid model ID".to_string())?,
                            ),
                            backdrop: gift
                                .backdrop
                                .map(|id| {
                                    choice(&choices.backdrop, i64::from(id)).and_then(|id| {
                                        id.try_into().map_err(|_| "Invalid backdrop ID".into())
                                    })
                                })
                                .transpose()?,
                            pattern: gift
                                .pattern
                                .map(|id| {
                                    choice(&choices.pattern, id.get()).and_then(|id| {
                                        id.try_into().map_err(|_| "Invalid pattern ID".into())
                                    })
                                })
                                .transpose()?,
                        },
                    })
                };
                convert()
            })
            .collect())
    }
}
