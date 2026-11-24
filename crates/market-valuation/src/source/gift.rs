//! Resolve public gift identities from observed listings and executions.
use super::*;
use market_gifts::*;
use market_hub_protocol::wire as w;
impl ValuationSnapshot {
    /// Match transport facts through catalog aliases, without assuming hub ID formats.
    pub(crate) fn telegram_gift(&self, facts: &TelegramGiftDetails) -> Result<CollectibleGift> {
        let mut gift = self.telegram_traits(facts)?;
        match self.gift(&GiftLookup::Catalog {
            collection_id: gift.collection_id.clone(),
            number: facts.number,
        }) {
            Ok(observed) => gift.id = observed.id,
            Err(e) if e.code() == tonic::Code::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(gift)
    }
    pub(crate) fn telegram_traits(&self, facts: &TelegramGiftDetails) -> Result<CollectibleGift> {
        self.check_cancelled()?;
        let items = market_replica::catalog_read::entities_cancellable(
            &self.reader,
            w::EntityKind::ItemType,
            None,
            &self.cancelled,
        )?;
        let lookup = ModelLookup::Telegram {
            gift_id: facts.collection,
            document_id: facts.model,
        };
        let item = market_gifts_hub::find_collection(
            &lookup,
            items.iter().filter_map(|item| match item {
                w::entity::Entity::ItemType(item) => Some(item),
                _ => None,
            }),
        )
        .map_err(|_| {
            Status::unavailable(
                "Telegram gift collection is missing or ambiguous in the market catalog",
            )
        })?;
        let rules = market_gifts_hub::CollectionChoices::new(item)
            .map_err(|_| Status::unavailable("Invalid gift collection catalog"))?;
        let collection_id: CollectionId = item
            .id
            .clone()
            .try_into()
            .map_err(|_| Status::unavailable("Invalid gift collection ID"))?;
        let choice = |kind: &str, id: i64| -> Result<String> {
            let ids = rules.trait_option_ids(kind);
            let options = market_replica::catalog_read::trait_options(
                &self.reader,
                kind,
                ids.as_deref(),
                &self.cancelled,
            )?;
            let mut found = options.iter().filter(|option| {
                rules.accepts(option) && market_gifts_hub::has_telegram_alias(id, &option.aliases)
            });
            let value = found.next().ok_or_else(|| {
                Status::unavailable("Telegram gift trait is not in the market catalog")
            })?;
            if found.next().is_some() {
                return Err(Status::unavailable(
                    "Telegram gift trait alias is ambiguous",
                ));
            }
            Ok(value.id.clone())
        };
        let traits = GiftTraits {
            model: Some(
                choice(market_gifts_hub::MODEL, facts.model.get())?
                    .try_into()
                    .map_err(|_| Status::unavailable("Invalid model ID"))?,
            ),
            backdrop: facts
                .backdrop
                .map(|id| {
                    choice(market_gifts_hub::BACKDROP, i64::from(id)).and_then(|id| {
                        id.try_into()
                            .map_err(|_| Status::unavailable("Invalid backdrop ID"))
                    })
                })
                .transpose()?,
            pattern: facts
                .pattern
                .map(|id| {
                    choice(market_gifts_hub::PATTERN, id.get()).and_then(|id| {
                        id.try_into()
                            .map_err(|_| Status::unavailable("Invalid pattern ID"))
                    })
                })
                .transpose()?,
        };
        let id = facts.id.clone();
        Ok(CollectibleGift {
            id,
            collection_id,
            number: Some(facts.number),
            traits,
        })
    }
    pub fn gift(&self, lookup: &GiftLookup) -> Result<CollectibleGift> {
        match lookup {
            GiftLookup::Slug(slug) => self.gift_by_slug(slug),
            GiftLookup::Catalog {
                collection_id,
                number,
            } => {
                let Some(w::entity::Entity::ItemType(item)) =
                    self.entity(w::EntityKind::ItemType, collection_id)?
                else {
                    return Err(Status::not_found(
                        "Gift collection is not in available market data",
                    ));
                };
                if !market_gifts_hub::is_gift(&item) {
                    return Err(Status::not_found(
                        "Gift collection is not in available market data",
                    ));
                }
                self.gift_by_number(&item, *number)
            }
        }
    }
    fn gift_by_slug(&self, slug: &GiftSlug) -> Result<CollectibleGift> {
        self.check_cancelled()?;
        let items = market_replica::catalog_read::entities_cancellable(
            &self.reader,
            w::EntityKind::ItemType,
            None,
            &self.cancelled,
        )?;
        let mut collection = None;
        for row in &items {
            let w::entity::Entity::ItemType(item) = row else {
                continue;
            };
            if !market_gifts_hub::is_gift(item) {
                continue;
            }
            let canonical = market_gifts_hub::collection_slug(item)
                .map_err(|e| Status::unavailable(e.to_string()))?;
            if canonical.as_ref() != Some(slug.collection()) {
                continue;
            }
            if collection.replace(item).is_some() {
                return Err(Status::invalid_argument(
                    "Gift collection slug is ambiguous",
                ));
            }
        }
        let item = collection.ok_or_else(|| {
            Status::not_found("Canonical gift collection slug is not in available market data")
        })?;
        self.gift_by_number(item, slug.number())
    }
    fn gift_by_number(&self, item: &w::ItemType, number: GiftNumber) -> Result<CollectibleGift> {
        if !item.attributes.contains_key(market_gifts_hub::NUMBER) {
            return Err(Status::not_found(
                "Gift number is unavailable in available market data",
            ));
        }
        let currencies = market_replica::catalog_read::entities_cancellable(
            &self.reader,
            w::EntityKind::Currency,
            None,
            &self.cancelled,
        )?;
        let mut found: Option<CollectibleGift> = None;
        let mut budget = market_replica::catalog_read::ScanBudget::default();
        for currency in currencies {
            let w::entity::Entity::Currency(currency) = currency else {
                continue;
            };
            let mut filters = GiftFilters::new(
                CollectionId::new(item.id.clone())
                    .map_err(|e| Status::unavailable(e.to_string()))?,
                CurrencyId::new(currency.id).map_err(|e| Status::unavailable(e.to_string()))?,
            );
            filters.number = Some(GiftNumberRange {
                min: Some(number),
                max: Some(number),
            });
            let mut query = GiftOrderQuery::new(filters);
            query.sort = Some(GiftSort::NumberAsc);
            query.limit = 200;
            for source in [OrderSource::Asks, OrderSource::History] {
                query.source = source;
                let mut continuation = String::new();
                loop {
                    self.check_cancelled()?;
                    budget.page()?;
                    let page = self.gifts(query.clone(), continuation)?;
                    for gift in page.data {
                        if gift.number != Some(number) {
                            continue;
                        }
                        match &mut found {
                            None => found = Some(gift),
                            Some(current) => {
                                if current.id != gift.id {
                                    return Err(Status::unavailable(
                                        "Conflicting gift identities for this collection and number",
                                    ));
                                }
                                merge(&mut current.traits.model, gift.traits.model)?;
                                merge(&mut current.traits.backdrop, gift.traits.backdrop)?;
                                merge(&mut current.traits.pattern, gift.traits.pattern)?;
                            }
                        }
                    }
                    if page.next_page_token.is_empty() {
                        break;
                    }
                    continuation = page.next_page_token;
                }
            }
        }
        found.ok_or_else(|| {
            Status::not_found(
                "Gift attributes are unknown in market listings and executions; supply collection and traits directly",
            )
        })
    }
}
fn merge<T: PartialEq>(current: &mut Option<T>, next: Option<T>) -> Result<()> {
    match (current.as_ref(), next) {
        (Some(current), Some(next)) if current != &next => Err(Status::unavailable(
            "Conflicting gift traits in market listings",
        )),
        (None, next) => {
            *current = next;
            Ok(())
        }
        _ => Ok(()),
    }
}
