use super::*;
use market_gifts::*;
use market_hub_protocol::wire as w;
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BookScope {
    pub collection: CollectionId,
    pub currency: CurrencyId,
}
pub(super) fn request(scope: &BookScope) -> w::ReadRequest {
    w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(w::Scope {
                item_type_id: Some(scope.collection.to_string()),
                currency_id: Some(scope.currency.to_string()),
                modes: vec![w::OrderMode::Ask as i32],
                ..Default::default()
            })),
            ..Default::default()
        }),
        mode: w::ReadMode::Live as i32,
        sort: w::ReadSort::PriceAsc as i32,
        limit: 500,
        ..Default::default()
    }
}
impl ValuationSnapshot {
    /// Bounded, domain-projected observations for target estimators. The snapshot stays pinned.
    pub fn valuation_orders(
        &self,
        query: &GiftOrderQuery,
        continuation: String,
    ) -> Result<Page<GiftOrder>> {
        self.check_cancelled()?;
        let mut request = market_gifts_hub::query::compile(query)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        request.page_token = continuation;
        let page = self
            .reader
            .read_checked(
                request,
                |_, _| Ok(()),
                |entity| {
                    self.check_cancelled()?;
                    if let Some(w::entity::Entity::Order(order)) = &entity.entity {
                        if query.source == OrderSource::Asks {
                            return market_gifts_hub::is_exhausted(order)
                                .map(|v| !v)
                                .map_err(|e| Status::data_loss(e.to_string()));
                        }
                    }
                    Ok(true)
                },
            )?
            .page;
        let data = page
            .entities
            .into_iter()
            .map(|entity| match entity.entity {
                Some(w::entity::Entity::Order(order)) => {
                    market_gifts_hub::order(order).map_err(|e| Status::data_loss(e.to_string()))
                }
                _ => Err(Status::data_loss("expected a gift observation")),
            })
            .collect::<Result<_>>()?;
        Ok(Page {
            data,
            next_page_token: page.next_page_token,
        })
    }
    pub(super) fn gifts(
        &self,
        query: GiftOrderQuery,
        continuation: String,
    ) -> Result<Page<CollectibleGift>> {
        self.check_cancelled()?;
        let mut request = market_gifts_hub::query::compile(&query)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        request.page_token = continuation;
        let page = self
            .reader
            .read_checked(
                request,
                |_, _| Ok(()),
                |entity| {
                    self.check_cancelled()?;
                    match entity.entity.as_ref() {
                        Some(w::entity::Entity::Order(order)) => {
                            market_gifts_hub::is_exhausted(order)
                                .map(|v| !v)
                                .map_err(|e| Status::data_loss(e.to_string()))
                        }
                        _ => Err(Status::data_loss("expected a gift order")),
                    }
                },
            )?
            .page;
        Ok(Page {
            next_page_token: page.next_page_token,
            data: page
                .entities
                .into_iter()
                .filter_map(|entity| match entity.entity {
                    Some(w::entity::Entity::Order(order)) => market_gifts_hub::concrete_gift(order)
                        .map_err(|e| Status::data_loss(e.to_string()))
                        .transpose(),
                    _ => Some(Err(Status::data_loss("expected a gift order"))),
                })
                .collect::<Result<_>>()?,
        })
    }
    /// One page of sorted, eligible asks; cancellation is checked at every page boundary.
    pub fn asks(
        &self,
        scope: &BookScope,
        now_ms: i64,
        page_token: String,
    ) -> Result<Page<market_gifts::AskQuote>> {
        self.check_cancelled()?;
        let mut query = request(scope);
        query.page_token = page_token;
        let page = self.reader.read(query)?.page;
        let mut data = Vec::new();
        for entity in page.entities {
            if let Some(w::entity::Entity::Order(order)) = entity.entity
                && let Some(quote) = market_gifts_hub::ask_quote(order, now_ms)
                    .map_err(|e| Status::unavailable(e.to_string()))?
            {
                data.push(quote);
            }
        }
        Ok(Page {
            data,
            next_page_token: page.next_page_token,
        })
    }
}
