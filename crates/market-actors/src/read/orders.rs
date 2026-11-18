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
impl MarketSnapshot {
    pub fn orders(&self, query: GiftOrdersQuery) -> Result<Page<GiftOrder>> {
        self.check_cancelled()?;
        let request = crate::query::orders(query, w::ReadMode::Live)?;
        crate::protocol::orders(self.reader.read_checked(
            request,
            crate::catalog::validate_order_collection,
            |entity| {
                self.check_cancelled()
                    .map_err(|e| tonic::Status::unavailable(e.to_string()))?;
                crate::protocol::include_order(entity)
            },
        )?)
    }
    /// One page of sorted, eligible asks; cancellation is checked at every page boundary.
    pub fn asks(
        &self,
        scope: &BookScope,
        now_ms: i64,
        page_token: String,
    ) -> Result<Page<AskQuote>> {
        self.check_cancelled()?;
        let mut query = request(scope);
        query.page_token = page_token;
        let page = self.reader.read(query)?.page;
        let mut data = Vec::new();
        for entity in page.entities {
            if let Some(w::entity::Entity::Order(order)) = entity.entity
                && let Some(quote) = market_gifts_hub::ask_quote(order, now_ms)
                    .map_err(|e| DataError::unavailable(e.to_string()))?
            {
                data.push(quote);
            }
        }
        Ok(Page {
            data,
            next_page_token: page.next_page_token,
            cursor: page.cursor,
            sync: self.sync()?,
        })
    }
}
