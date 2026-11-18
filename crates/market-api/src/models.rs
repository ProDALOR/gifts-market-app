//! Gift domain models plus the HTTP/WebSocket envelope. No public generic hub DTOs.
//! Internal orders are intentionally absent from this public presentation module.
//! ```compile_fail
//! use market_api::models::GiftOrder;
//! ```
pub use crate::catalog::{
    GiftCatalog, GiftCollection, GiftCollectionCatalog, GiftMedia, GiftModel, GiftPattern,
};
pub use crate::orders::{
    OrderPageResponse, OrderResponse, OrderTargetResponse, OrderWindowResponse,
    SnapshotWindowResponse, UnsupportedTargetReason, WindowOrderResponse,
};
pub use market_gifts::{
    Amount, BackdropId, BackdropPalette, CatalogVersions, CollectibleGift, CollectionId,
    CollectionSlug, Currency, CurrencyId, GiftBackdrop, GiftBook, GiftCriteria, GiftFilters,
    GiftHistoryQuery, GiftNumber, GiftNumberRange, GiftOrderKind, GiftOrderWindowQuery,
    GiftOrdersQuery, GiftSide, GiftSlug, GiftSort, GiftTraits, ModelId, OrderId, PatternId, Period,
    PriceRange, ReadConsistency, SyncState, SyncStatus, UnsupportedGiftReason, Venue, VenueId,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Request(ApiRequest),
    SubscribeWindow {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        query: Box<market_gifts::GiftOrderWindowQuery>,
    },
    Cancel {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
    },
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
    #[serde(rename = "valuation.evaluate")]
    Valuation {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        params: Box<market_valuation::GiftValuationRequest>,
    },
}
impl ApiRequest {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Valuation { id, .. } => id,
        }
    }
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame<'a> {
    Result(ApiResult<'a>),
    /// Rejected command; the referenced operation continues unchanged.
    CommandError {
        id: String,
        error: crate::error::ApiError,
    },
    Window {
        id: String,
        window: crate::orders::OrderWindowResponse<'a>,
        sync: SyncStatus,
        revision: String,
    },
    Status {
        id: String,
        state: SubscriptionStatus,
    },
    Sync {
        id: String,
        sync: SyncStatus,
        revision: String,
    },
    Catalog {
        id: String,
        versions: CatalogVersions,
    },
    Error {
        id: String,
        error: crate::error::ApiError,
    },
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "method")]
pub enum ApiResult<'a> {
    #[serde(rename = "valuation.evaluate")]
    Valuation {
        #[schema(min_length = 1, max_length = 64, pattern = "^[A-Za-z0-9_.-]+$")]
        id: String,
        data: GiftValuationResponse<'a>,
    },
}
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    Connecting,
}

pub use crate::valuation::GiftValuationResponse;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_requests_keep_flat_wire_envelopes_and_reject_unknown_methods_and_parameters() {
        let value = serde_json::json!({"type":"request","id":"request-1","method":"valuation.evaluate","params":{"gift":"Gift-123","currency_id":"GRAM"}});
        let request: Command = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(
            request,
            Command::Request(ApiRequest::Valuation { .. })
        ));
        assert_eq!(serde_json::to_value(request).unwrap(), value);
        for invalid in [
            serde_json::json!({"type":"subscribe","id":"old","query":{"filters":{"collection_id":"gift","currency_id":"GRAM"}}}),
            serde_json::json!({"type":"unsubscribe","id":"old"}),
            serde_json::json!({"type":"request","id":"x","method":"valuation.evaluate","params":{"gift":"Gift-123","currency_id":"GRAM"},"extra":true}),
            serde_json::json!({"type":"cancel","id":"x","extra":true}),
            serde_json::json!({"type":"request","id":"x","method":"arbitrary.invoke","params":{}}),
            serde_json::json!({"type":"request","id":"x","method":"valuation.evaluate","params":{"gift":"Gift-123","currency_id":"GRAM","extra":true}}),
        ] {
            assert!(serde_json::from_value::<Command>(invalid).is_err());
        }
    }
}
