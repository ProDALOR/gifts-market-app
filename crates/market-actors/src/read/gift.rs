//! Resolve public slugs or stable collection/number identities from synchronized orders.
use super::*;
use market_gifts::*;
use market_hub_protocol::wire as w;
impl MarketSnapshot {
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
                    return Err(DataError::not_found(
                        "Gift collection is not in synchronized market data",
                    ));
                };
                if !market_gifts_hub::is_gift(&item) {
                    return Err(DataError::not_found(
                        "Gift collection is not in synchronized market data",
                    ));
                }
                self.gift_by_number(&item, *number)
            }
        }
    }
    fn gift_by_slug(&self, slug: &GiftSlug) -> Result<CollectibleGift> {
        self.check_cancelled()?;
        let items = crate::catalog::projection::entities_cancellable(
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
                .map_err(|e| DataError::unavailable(e.to_string()))?;
            if canonical.as_ref() != Some(slug.collection()) {
                continue;
            }
            if collection.replace(item).is_some() {
                return Err(DataError::invalid("Gift collection slug is ambiguous"));
            }
        }
        let item = collection.ok_or_else(|| {
            DataError::not_found(
                "Canonical gift collection slug is not in synchronized market data",
            )
        })?;
        self.gift_by_number(item, slug.number())
    }
    fn gift_by_number(&self, item: &w::ItemType, number: GiftNumber) -> Result<CollectibleGift> {
        if !item.attributes.contains_key(market_gifts_hub::NUMBER) {
            return Err(DataError::not_found(
                "Gift number is unavailable in synchronized market data",
            ));
        }
        let currencies = crate::catalog::projection::entities_cancellable(
            &self.reader,
            w::EntityKind::Currency,
            None,
            &self.cancelled,
        )?;
        let mut found: Option<CollectibleGift> = None;
        for currency in currencies {
            let w::entity::Entity::Currency(currency) = currency else {
                continue;
            };
            let mut filters = GiftFilters::new(
                CollectionId::new(item.id.clone())
                    .map_err(|e| DataError::unavailable(e.to_string()))?,
                CurrencyId::new(currency.id).map_err(|e| DataError::unavailable(e.to_string()))?,
            );
            filters.number = Some(GiftNumberRange {
                min: Some(number),
                max: Some(number),
            });
            let mut query = GiftOrdersQuery::new(filters);
            query.sort = GiftSort::NumberAsc;
            query.limit = 200;
            loop {
                self.check_cancelled()?;
                let page = self.orders(query.clone())?;
                for projected in page.data {
                    let GiftTarget::Gift { gift } = projected.into_data().target else {
                        continue;
                    };
                    if gift.number != Some(number) {
                        continue;
                    }
                    match &mut found {
                        None => found = Some(gift),
                        Some(current) => {
                            if current.id != gift.id {
                                return Err(DataError::unavailable(
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
                query.page_token = page.next_page_token;
            }
        }
        found.ok_or_else(|| {
            DataError::not_found(
                "Gift is not in synchronized market asks; unlisted gifts cannot be resolved",
            )
        })
    }
}
fn merge<T: PartialEq>(current: &mut Option<T>, next: Option<T>) -> Result<()> {
    match (current.as_ref(), next) {
        (Some(current), Some(next)) if current != &next => Err(DataError::unavailable(
            "Conflicting gift traits in synchronized market orders",
        )),
        (None, next) => {
            *current = next;
            Ok(())
        }
        _ => Ok(()),
    }
}
