use crate::status::{ResponseMeta, StatusData};
use market_view::{
    CatalogEntry, CurrencyId, FeedId, FeedStatus, Item, ItemId, ItemTypeId, Order, OrderId,
    OrderSide,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Clone, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type=Vec<market_json::schema::Item>)]
pub struct Items(#[serde(serialize_with = "market_json::serialize_items")] pub Vec<Item>);
#[derive(Clone, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type=Vec<market_json::schema::Order>)]
pub struct Orders(#[serde(serialize_with = "market_json::serialize_orders")] pub Vec<Order>);
#[derive(Clone, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type=Vec<market_json::schema::CatalogEntry>)]
pub struct CatalogEntries(
    #[serde(serialize_with = "market_json::serialize_catalog")] pub Vec<CatalogEntry>,
);
#[derive(Clone, Serialize, ToSchema)]
pub struct Page {
    pub next_cursor: Option<String>,
}
#[derive(Clone, Default, Serialize, ToSchema)]
pub struct Included {
    #[serde(
        serialize_with = "market_json::serialize_items",
        skip_serializing_if = "Vec::is_empty"
    )]
    #[schema(value_type=Vec<market_json::schema::Item>)]
    pub items: Vec<Item>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[schema(value_type=Vec<market_json::schema::FeedStatus>)]
    pub feeds: Vec<FeedStatus>,
}
#[derive(Clone, Serialize, ToSchema)]
pub struct Response<T> {
    pub data: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<Page>,
    pub meta: ResponseMeta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<Included>,
}
pub type StatusResponse = Response<StatusData>;
pub type CatalogResponse = Response<CatalogEntries>;
pub type ItemsResponse = Response<Items>;
pub type OrdersResponse = Response<Orders>;
#[derive(Clone, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type=Vec<market_json::schema::FeedStatus>)]
pub struct Feeds(pub Vec<FeedStatus>);
pub type FeedsResponse = Response<Feeds>;
#[derive(Clone, Serialize, ToSchema)]
pub struct BookId {
    #[schema(value_type=String)]
    pub item_type_id: ItemTypeId,
    #[schema(value_type=String)]
    pub currency_id: CurrencyId,
}
#[derive(Clone, Serialize, ToSchema)]
pub struct OrderPage {
    pub data: Orders,
    pub page: Page,
}
#[derive(Clone, Serialize, ToSchema)]
pub struct BookData {
    pub book: BookId,
    pub asks: OrderPage,
    pub bids: OrderPage,
}
pub type BookResponse = Response<BookData>;

#[derive(Clone, Default, Debug, Serialize, Deserialize, IntoParams)]
#[serde(default, deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct CatalogQuery {
    /// venue, currency, feed, item_type, attribute, or attribute_value.
    pub kind: Option<String>,
    pub id: Option<String>,
    #[param(value_type=Option<String>)]
    pub item_type_id: Option<ItemTypeId>,
    #[param(value_type=Option<String>)]
    pub attribute_id: Option<market_view::AttributeId>,
    pub q: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}
#[derive(Clone, Default, Debug, Serialize, Deserialize, IntoParams)]
#[serde(default, deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct ItemsQuery {
    #[param(value_type=Option<String>)]
    pub feed_id: Option<FeedId>,
    #[param(value_type=Option<String>)]
    pub item_type_id: Option<ItemTypeId>,
    #[param(value_type=Option<String>)]
    pub item_id: Option<ItemId>,
    /// Catalog-aware JSON map of exact candidate attributes.
    pub attributes: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}
#[derive(Clone, Default, Debug, Serialize, Deserialize, IntoParams)]
#[serde(default, deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct FeedsQuery {
    #[param(value_type=Option<String>)]
    pub feed_id: Option<FeedId>,
}
#[derive(Clone, Default, Debug, Serialize, Deserialize, IntoParams)]
#[serde(default, deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct OrdersQuery {
    /// Required together with currency_id for price-ordered market queries.
    #[param(value_type=Option<String>)]
    pub item_type_id: Option<ItemTypeId>,
    #[param(value_type=Option<String>)]
    pub currency_id: Option<CurrencyId>,
    #[param(value_type=Option<market_json::schema::OrderSide>)]
    pub side: Option<OrderSide>,
    #[param(value_type=Option<String>)]
    pub feed_id: Option<FeedId>,
    #[param(value_type=Option<String>)]
    pub item_id: Option<ItemId>,
    /// Lookup requires feed_id; side disambiguates a venue that reuses an ID.
    #[param(value_type=Option<String>)]
    pub order_id: Option<OrderId>,
    pub attributes: Option<String>,
    pub price_min: Option<String>,
    pub price_max: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
    /// Comma-separated item,feed dependencies from the same view.
    pub include: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct BookQuery {
    #[param(value_type=String)]
    pub item_type_id: ItemTypeId,
    #[param(value_type=String)]
    pub currency_id: CurrencyId,
    #[param(value_type=Option<String>)]
    pub feed_id: Option<FeedId>,
    #[param(value_type=Option<String>)]
    pub item_id: Option<ItemId>,
    pub attributes: Option<String>,
    pub price_min: Option<String>,
    pub price_max: Option<String>,
    pub limit: Option<usize>,
    pub include: Option<String>,
}
#[derive(Clone, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type=Vec<market_json::schema::Trade>)]
pub struct Trades(pub Vec<serde_json::Value>);
pub type TradesResponse = Response<Trades>;
#[derive(Clone, Default, Debug, Serialize, Deserialize, IntoParams)]
#[serde(default, deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct TradesQuery {
    #[param(value_type=Option<String>)]
    pub feed_id: Option<FeedId>,
    #[param(value_type=Option<String>)]
    pub item_type_id: Option<ItemTypeId>,
    #[param(value_type=Option<String>)]
    pub currency_id: Option<CurrencyId>,
    #[param(value_type=Option<String>)]
    pub item_id: Option<ItemId>,
    #[param(value_type=Option<String>,format=DateTime)]
    pub executed_from: Option<market_view::MarketTimestamp>,
    #[param(value_type=Option<String>,format=DateTime)]
    pub executed_until: Option<market_view::MarketTimestamp>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}
