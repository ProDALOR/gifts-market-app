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
/// Gift projection over scoped generic active-order partitions, with no request token retained.
pub(crate) struct AskBookSnapshot(market_replica::OrderBookSnapshot);
impl AskBookSnapshot {
    pub(crate) fn pair_quotes<'a>(
        &'a self,
        model: &ModelId,
        backdrop: &BackdropId,
        now_ms: i64,
    ) -> impl Iterator<Item = Result<Option<AskQuote>>> + use<'a> {
        let attributes = [
            ("tg.model", model.as_str()),
            ("tg.backdrop", backdrop.as_str()),
        ]
        .into_iter()
        .map(|(key, value)| {
            (
                key.to_owned(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId(value.to_owned())),
                    })),
                },
            )
        })
        .collect();
        let model = model.clone();
        let backdrop = backdrop.clone();
        self.0.candidates(attributes).map(move |entity| {
            let Some(w::entity::Entity::Order(order)) = &entity.entity else {
                return Err(Status::data_loss("expected an active order"));
            };
            market_gifts_hub::ask_quote(order, now_ms)
                .map(|quote| {
                    quote.filter(|quote| {
                        quote.model_id.as_ref() == Some(&model)
                            && quote.backdrop_id.as_ref() == Some(&backdrop)
                    })
                })
                .map_err(|error| Status::data_loss(error.to_string()))
        })
    }
}
fn read_options(query: &GiftOrderQuery) -> market_replica::ReadOptions {
    use market_replica::Sort;
    market_replica::ReadOptions {
        sort: Some(match query.sort() {
            market_gifts::GiftSort::Identity => Sort::Identity,
            market_gifts::GiftSort::PriceAsc => Sort::PriceAsc,
            market_gifts::GiftSort::PriceDesc => Sort::PriceDesc,
            market_gifts::GiftSort::TimeAsc => Sort::TimeAsc,
            market_gifts::GiftSort::TimeDesc => Sort::TimeDesc,
            market_gifts::GiftSort::NumberAsc => Sort::NumberAsc,
            market_gifts::GiftSort::NumberDesc => Sort::NumberDesc,
        }),
        ..Default::default()
    }
}
impl ValuationSnapshot {
    pub(crate) fn ask_book(&self, scope: &BookScope) -> Result<AskBookSnapshot> {
        self.check_cancelled()?;
        self.reader
            .order_book_snapshot(
                scope.collection.as_str(),
                scope.currency.as_str(),
                w::OrderMode::Ask,
            )
            .map(AskBookSnapshot)
    }
    /// Bounded, domain-projected observations for target estimators. The snapshot stays pinned.
    pub(crate) fn valuation_orders(
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
            .read_checked_with_options(
                request,
                read_options(query),
                |_, _| Ok(()),
                |entity| {
                    self.check_cancelled()?;
                    if let Some(w::entity::Entity::Order(order)) = &entity.entity
                        && query.source == OrderSource::Asks
                    {
                        return market_gifts_hub::is_exhausted(order)
                            .map(|v| !v)
                            .map_err(|e| Status::data_loss(e.to_string()));
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
            .read_checked_with_options(
                request,
                read_options(&query),
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
        let mut data = Vec::with_capacity(page.entities.len());
        for entity in page.entities {
            self.check_cancelled()?;
            if let Some(w::entity::Entity::Order(order)) = entity.entity
                && let Some(quote) = market_gifts_hub::ask_quote(&order, now_ms)
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
