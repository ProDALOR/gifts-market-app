//! Batch Telegram aliases; unrestricted trait directories are read at most once per batch.
use super::*;
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::collections::{BTreeMap, BTreeSet};
type Aliases = BTreeMap<i64, Option<String>>;
#[derive(Default)]
struct Needed {
    model: BTreeSet<i64>,
    backdrop: BTreeSet<i64>,
    pattern: BTreeSet<i64>,
}
impl Needed {
    fn insert(&mut self, gift: &market_gifts::TelegramGiftDetails) {
        self.model.insert(gift.model.get());
        self.backdrop.extend(gift.backdrop.map(i64::from));
        self.pattern
            .extend(gift.pattern.map(TelegramDocumentId::get));
    }
}
struct Choices {
    collection: CollectionId,
    model: Arc<Aliases>,
    backdrop: Arc<Aliases>,
    pattern: Arc<Aliases>,
}
fn aliases(
    options: impl IntoIterator<Item = w::AttributeOption>,
    needed: &BTreeSet<i64>,
    cancelled: &tokio_util::sync::CancellationToken,
) -> Result<Arc<Aliases>> {
    let mut map: Aliases = BTreeMap::new();
    for option in options {
        market_replica::catalog_read::check(cancelled)?;
        for alias in
            market_gifts_hub::telegram_alias_ids(&option.aliases).filter(|id| needed.contains(id))
        {
            map.entry(alias)
                .and_modify(|old| {
                    if old.as_ref() != Some(&option.id) {
                        *old = None;
                    }
                })
                .or_insert_with(|| Some(option.id.clone()));
        }
    }
    Ok(Arc::new(map))
}
fn choice(map: &Aliases, id: i64) -> std::result::Result<String, String> {
    map.get(&id)
        .and_then(Option::as_ref)
        .cloned()
        .ok_or_else(|| "Gift trait is missing or ambiguous in the catalog".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_aliases_preserve_ambiguity_and_cancellation() {
        let option = |id: &str, alias: i64| w::AttributeOption {
            id: id.into(),
            aliases: vec![w::ExternalId {
                namespace: "tg".into(),
                id: alias.to_string(),
            }],
            ..Default::default()
        };
        let token = CancellationToken::new();
        let map = aliases(
            [
                option("a", 1),
                option("a", 1),
                option("b", 2),
                option("c", 2),
                option("d", 3),
            ],
            &[1, 2].into(),
            &token,
        )
        .unwrap();
        assert_eq!(choice(&map, 1).unwrap(), "a");
        assert!(choice(&map, 2).is_err());
        assert!(
            !map.contains_key(&3),
            "unrequested aliases are not retained"
        );
        token.cancel();
        assert!(aliases([option("a", 1)], &[1].into(), &token).is_err());
    }
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
        let mut wanted: BTreeMap<TelegramGiftId, Needed> = BTreeMap::new();
        let mut all = Needed::default();
        for gift in gifts {
            wanted
                .entry(gift.gift.collection)
                .or_default()
                .insert(&gift.gift);
            all.insert(&gift.gift);
        }
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
                        && wanted.contains_key(&key)
                    {
                        collections
                            .entry(id)
                            .and_modify(|v| *v = None)
                            .or_insert(Some(item));
                    }
                }
            }
        }
        let mut unrestricted = BTreeMap::new();
        let mut choices = BTreeMap::new();
        for (id, needed) in wanted {
            self.check_cancelled()?;
            if let Some(Some(item)) = collections.get(&id.get()) {
                let rules = market_gifts_hub::CollectionChoices::new(item)
                    .map_err(|_| Status::unavailable("Invalid collection catalog"))?;
                let mut maps = Vec::with_capacity(3);
                for (kind, needed, all) in [
                    (market_gifts_hub::MODEL, &needed.model, &all.model),
                    (market_gifts_hub::BACKDROP, &needed.backdrop, &all.backdrop),
                    (market_gifts_hub::PATTERN, &needed.pattern, &all.pattern),
                ] {
                    self.check_cancelled()?;
                    let map = if needed.is_empty() {
                        Arc::new(Aliases::new())
                    } else if let Some(ids) = rules.trait_option_ids(kind) {
                        let options = market_replica::catalog_read::trait_options(
                            &self.reader,
                            kind,
                            Some(&ids),
                            &self.cancelled,
                        )?;
                        aliases(
                            options.into_iter().filter(|v| rules.accepts(v)),
                            needed,
                            &self.cancelled,
                        )?
                    } else {
                        // This rule accepts every option of the kind. Share only requested
                        // aliases, rather than cloning the entire directory for each collection.
                        if !unrestricted.contains_key(kind) {
                            let options = market_replica::catalog_read::trait_options(
                                &self.reader,
                                kind,
                                None,
                                &self.cancelled,
                            )?;
                            unrestricted.insert(kind, aliases(options, all, &self.cancelled)?);
                        }
                        unrestricted[kind].clone()
                    };
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
        gifts
            .iter()
            .map(|item| {
                self.check_cancelled()?;
                let gift = &item.gift;
                let result = (|| -> std::result::Result<CollectibleGift, String> {
                    let choices = choices.get(&gift.collection).ok_or_else(|| {
                        "Gift collection is missing or ambiguous in the catalog".to_string()
                    })?;
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
                })();
                Ok(result)
            })
            .collect()
    }
}
