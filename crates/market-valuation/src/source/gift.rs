//! Resolve public slugs or stable collection/number identities from synchronized orders.
use super::*;
use market_gifts::*;
use market_hub_protocol::wire as w;
impl ValuationSnapshot {
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
        found.ok_or_else(|| {
            Status::not_found(
                "Gift is not in available market asks; unlisted gifts cannot be resolved",
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
