//! Gift domain models plus the HTTP/WebSocket envelope. No public generic hub DTOs.
//! Internal orders are intentionally absent from this public presentation module.
//! ```compile_fail
//! use market_api::models::GiftOrder;
//! ```
pub use crate::catalog::{
    GiftCatalog, GiftCollection, GiftCollectionCatalog, GiftMedia, GiftModel, GiftPattern,
};
pub use crate::orders::{
    OrderIdentity, OrderReadResponse, OrderResponse, OrderSnapshotResponse, OrderTargetResponse,
    OrderedOrderResponse,
};
pub use crate::status::MarketStatus;
pub use market_gifts::{
    Amount, BackdropId, BackdropPalette, BidMatch, CatalogVersions, CollectibleGift, CollectionId,
    CollectionSlug, Currency, CurrencyId, GiftBackdrop, GiftBook, GiftCriteria, GiftFilters,
    GiftNumber, GiftNumberRange, GiftOrderKind, GiftOrderQuery, GiftSide, GiftSlug, GiftSort,
    GiftTraits, ModelId, OrderId, PatternId, Period, PriceRange, UnsupportedGiftReason, Venue,
    VenueId,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    #[serde(rename = "portfolio.watch")]
    WatchPortfolio {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        params: market_portfolio::PortfolioQuery,
    },
    #[serde(rename = "valuation.watch")]
    WatchValuation {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        params: Box<market_valuation::ValuationQuery>,
    },
    Request(ApiRequest),
    #[serde(rename = "orders.watch")]
    WatchOrders {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        params: Box<market_gifts::GiftOrderQuery>,
    },
    Cancel {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
    },
    #[serde(rename = "catalog.watch")]
    WatchCatalog {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        collection_ids: Vec<CollectionId>,
    },
}
/// Finite, typed request methods; each method owns its parameters.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "method", deny_unknown_fields)]
pub enum ApiRequest {
    #[serde(rename = "portfolio.query")]
    PortfolioQuery {
        id: String,
        params: market_portfolio::PortfolioQuery,
    },
    #[serde(rename = "portfolio.refresh")]
    PortfolioRefresh {
        id: String,
        params: market_portfolio::PortfolioQuery,
    },
    #[serde(rename = "valuation.query")]
    Valuation {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        params: Box<market_valuation::ValuationQuery>,
    },
    #[serde(rename = "orders.query")]
    Orders {
        id: String,
        params: Box<market_gifts::GiftOrderQuery>,
    },
}
impl ApiRequest {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Orders { id, .. }
            | Self::Valuation { id, .. }
            | Self::PortfolioQuery { id, .. }
            | Self::PortfolioRefresh { id, .. } => id,
        }
    }
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame<'a> {
    #[serde(rename = "portfolio.snapshot")]
    PortfolioSnapshot {
        id: String,
        data: &'a market_portfolio::PortfolioState,
    },
    #[serde(rename = "portfolio.progress")]
    PortfolioProgress {
        id: String,
        refresh_id: u64,
        progress: &'a market_portfolio::PortfolioProgress,
        #[schema(value_type=Option<String>,format=DateTime)]
        refresh_after: Option<jiff::Timestamp>,
        error: Option<&'a str>,
    },
    #[serde(rename = "valuation.snapshot")]
    ValuationSnapshot {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        data: crate::valuation::ValuationResponse<'a>,
    },
    Result(ApiResult<'a>),
    /// Rejected command; active operations continue unchanged. Empty ID if no valid ID was parsed.
    CommandError {
        id: String,
        error: crate::error::ApiError,
    },
    #[serde(rename = "orders.snapshot")]
    OrdersSnapshot {
        id: String,
        sequence: u64,
        #[serde(flatten)]
        data: crate::orders::OrderSnapshotResponse<'a>,
    },
    #[serde(rename = "orders.changed")]
    OrdersChanged {
        id: String,
        sequence: u64,
        upserts: Vec<crate::orders::OrderedOrderResponse<'a>>,
        removed: Vec<crate::orders::OrderIdentity<'a>>,
    },
    #[serde(rename = "market.status")]
    MarketStatus {
        status: MarketStatus,
    },
    /// Compact opaque content tokens matching unquoted catalog HTTP ETags.
    #[serde(rename = "catalog.versions")]
    Catalog {
        id: String,
        versions: CatalogVersions,
    },
    Error {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        error: crate::error::ApiError,
    },
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "method")]
pub enum ApiResult<'a> {
    #[serde(rename = "portfolio.query")]
    PortfolioQuery {
        id: String,
        data: &'a market_portfolio::PortfolioState,
    },
    #[serde(rename = "portfolio.refresh")]
    PortfolioRefresh {
        id: String,
        data: &'a market_portfolio::PortfolioState,
    },
    #[serde(rename = "valuation.query")]
    Valuation {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        data: crate::valuation::ValuationResponse<'a>,
    },
    #[serde(rename = "orders.query")]
    Orders {
        id: String,
        #[serde(flatten)]
        data: crate::orders::OrderReadResponse<'a>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_requests_keep_flat_wire_envelopes_and_reject_unknown_methods_and_parameters() {
        let value = serde_json::json!({"type":"request","id":"request-1","method":"valuation.query","params":{"target":{"kind":"gift","gift":"Gift-123"},"strategies":["gift_floor"],"currency_id":"GRAM"}});
        let request: Command = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(
            request,
            Command::Request(ApiRequest::Valuation { .. })
        ));
        assert_eq!(serde_json::to_value(request).unwrap(), value);
        let catalog =
            serde_json::json!({"type":"catalog.watch","id":"catalog","collection_ids":[]});
        let command: Command = serde_json::from_value(catalog.clone()).unwrap();
        assert_eq!(serde_json::to_value(command).unwrap(), catalog);
        for invalid in [
            serde_json::json!({"type":"floors.watch","id":"old","params":{"currency_id":"GRAM","collection_ids":["gift"]}}),
            serde_json::json!({"type":"watch_catalog","id":"old","collection_ids":[]}),
            serde_json::json!({"type":"subscribe","id":"old","query":{"filters":{"collection_id":"gift","currency_id":"GRAM"}}}),
            serde_json::json!({"type":"unsubscribe","id":"old"}),
            serde_json::json!({"type":"request","id":"x","method":"valuation.query","params":{"target":{"kind":"gift","gift":"Gift-123"},"strategies":["gift_floor"],"currency_id":"GRAM"},"extra":true}),
            serde_json::json!({"type":"cancel","id":"x","extra":true}),
            serde_json::json!({"type":"request","id":"x","method":"arbitrary.invoke","params":{}}),
            serde_json::json!({"type":"request","id":"x","method":"valuation.query","params":{"target":{"kind":"gift","gift":"Gift-123"},"strategies":["gift_floor"],"currency_id":"GRAM","extra":true}}),
        ] {
            assert!(serde_json::from_value::<Command>(invalid).is_err());
        }
    }
}
