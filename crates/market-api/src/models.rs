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
pub use crate::protocol::OperationId;
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
        id: OperationId,
        params: market_portfolio::PortfolioQuery,
    },
    #[serde(rename = "valuation.watch")]
    WatchValuation {
        id: OperationId,
        params: Box<market_valuation::ValuationQuery>,
    },
    Request(ApiRequest),
    #[serde(rename = "orders.watch")]
    WatchOrders {
        id: OperationId,
        params: Box<market_gifts::GiftOrderQuery>,
    },
    Cancel {
        id: OperationId,
    },
    #[serde(rename = "catalog.watch")]
    WatchCatalog {
        id: OperationId,
        collection_ids: Vec<CollectionId>,
    },
}
/// Finite, typed request methods; each method owns its parameters.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "method", deny_unknown_fields)]
pub enum ApiRequest {
    #[serde(rename = "portfolio.query")]
    PortfolioQuery {
        id: OperationId,
        params: market_portfolio::PortfolioReadQuery,
    },
    #[serde(rename = "portfolio.refresh")]
    PortfolioRefresh {
        id: OperationId,
        params: market_portfolio::PortfolioQuery,
    },
    #[serde(rename = "valuation.batch")]
    ValuationBatch {
        id: OperationId,
        params: Box<market_valuation::ValuationBatchQuery>,
    },
    #[serde(rename = "valuation.query")]
    Valuation {
        id: OperationId,
        params: Box<market_valuation::ValuationQuery>,
    },
    #[serde(rename = "orders.query")]
    Orders {
        id: OperationId,
        params: Box<market_gifts::GiftOrderQuery>,
    },
}
impl ApiRequest {
    pub(crate) fn id(&self) -> &OperationId {
        match self {
            Self::Orders { id, .. }
            | Self::Valuation { id, .. }
            | Self::ValuationBatch { id, .. }
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
        id: &'a OperationId,
        data: crate::portfolio::PortfolioResponse<'a>,
    },
    #[serde(rename = "portfolio.progress")]
    PortfolioProgress {
        id: &'a OperationId,
        refresh_id: u64,
        progress: &'a market_portfolio::PortfolioProgress,
        #[schema(value_type=Option<String>,format=DateTime)]
        refresh_after: Option<jiff::Timestamp>,
        error: Option<&'a str>,
    },
    #[serde(rename = "valuation.snapshot")]
    ValuationSnapshot {
        id: &'a OperationId,
        data: crate::valuation::ValuationResponse<'a>,
    },
    Result(ApiResult<'a>),
    /// Rejected command; active operations continue unchanged. Empty ID if no valid ID was parsed.
    CommandError {
        #[serde(serialize_with = "crate::protocol::serialize_correlation")]
        #[schema(value_type = String, required = true, max_length = 64, pattern = "^[A-Za-z0-9_.-]*$")]
        id: Option<&'a OperationId>,
        error: &'a crate::error::ApiError,
    },
    #[serde(rename = "orders.snapshot")]
    OrdersSnapshot {
        id: &'a OperationId,
        sequence: u64,
        #[serde(flatten)]
        data: crate::orders::OrderSnapshotResponse<'a>,
    },
    #[serde(rename = "orders.changed")]
    OrdersChanged {
        id: &'a OperationId,
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
        id: &'a OperationId,
        versions: CatalogVersions,
    },
    Error {
        id: &'a OperationId,
        error: &'a crate::error::ApiError,
    },
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "method")]
pub enum ApiResult<'a> {
    #[serde(rename = "portfolio.query")]
    PortfolioQuery {
        id: &'a OperationId,
        data: &'a market_portfolio::PortfolioPage,
    },
    #[serde(rename = "portfolio.refresh")]
    PortfolioRefresh {
        id: &'a OperationId,
        data: crate::portfolio::PortfolioResponse<'a>,
    },
    #[serde(rename = "valuation.batch")]
    ValuationBatch {
        id: &'a OperationId,
        data: crate::valuation::ValuationBatchResponse<'a>,
    },
    #[serde(rename = "valuation.query")]
    Valuation {
        id: &'a OperationId,
        data: crate::valuation::ValuationResponse<'a>,
    },
    #[serde(rename = "orders.query")]
    Orders {
        id: &'a OperationId,
        #[serde(flatten)]
        data: crate::orders::OrderReadResponse<'a>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portfolio_user_schema_and_decode_share_numeric_bounds() {
        use market_portfolio::UserId;
        let schema = serde_json::to_value(<UserId as utoipa::PartialSchema>::schema()).unwrap();
        assert_eq!(schema["type"], "integer");
        let low = schema["minimum"].as_u64().unwrap();
        let high = schema["maximum"].as_u64().unwrap();
        for value in [low, high] {
            let user: UserId = serde_json::from_value(serde_json::json!(value)).unwrap();
            assert_eq!(
                serde_json::to_value(user).unwrap(),
                serde_json::json!(value)
            );
        }
        for value in [low - 1, high + 1] {
            assert!(serde_json::from_value::<UserId>(serde_json::json!(value)).is_err());
        }
    }

    #[test]
    fn command_error_ids_are_typed_optional_in_rust_but_required_strings_on_the_wire() {
        let schema =
            serde_json::to_value(<ServerFrame<'_> as utoipa::PartialSchema>::schema()).unwrap();
        let frame = schema["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["properties"]["type"]["enum"] == serde_json::json!(["command_error"]))
            .unwrap();
        assert_eq!(frame["properties"]["id"]["type"], "string");
        assert!(
            frame["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("id"))
        );
        let error = crate::error::ApiError::invalid("invalid API message");
        for (id, expected) in [(None, ""), (Some(&OperationId::from("op.1")), "op.1")] {
            let frame =
                serde_json::to_value(ServerFrame::CommandError { id, error: &error }).unwrap();
            assert_eq!(frame["id"], expected);
        }
    }
    #[test]
    fn every_command_uses_the_same_validated_string_id() {
        use serde_json::json;
        let selection = json!({"target":{"kind":"selection","collection_id":"gift","criteria":{}},"currency_id":"GRAM","strategies":["ask_floor"]});
        let orders =
            json!({"filters":{"collection_id":"gift","currency_id":"GRAM"},"source":"asks"});
        let portfolio = json!({"user_id":7,"currency_id":"GRAM"});
        for mut command in [
            json!({"type":"orders.watch","params":orders}),
            json!({"type":"valuation.watch","params":selection}),
            json!({"type":"portfolio.watch","params":portfolio}),
            json!({"type":"catalog.watch","collection_ids":[]}),
            json!({"type":"cancel"}),
            json!({"type":"request","method":"orders.query","params":orders}),
            json!({"type":"request","method":"valuation.query","params":selection}),
            json!({"type":"request","method":"portfolio.query","params":portfolio}),
            json!({"type":"request","method":"portfolio.refresh","params":portfolio}),
        ] {
            // Query defaults are unrelated to ID encoding; compare canonical payloads.
            command["id"] = json!("op");
            command =
                serde_json::to_value(serde_json::from_value::<Command>(command).unwrap()).unwrap();
            for id in [json!("op_1.next-2"), json!("x".repeat(64))] {
                command["id"] = id;
                let decoded: Command = serde_json::from_value(command.clone()).unwrap();
                assert_eq!(serde_json::to_value(decoded).unwrap(), command);
            }
            for id in [
                json!(""),
                json!("x".repeat(65)),
                json!("bad id"),
                json!("😀"),
                json!(null),
                json!(123),
                json!({"value":"x"}),
            ] {
                command["id"] = id;
                assert!(
                    serde_json::from_value::<Command>(command.clone()).is_err(),
                    "{command}"
                );
            }
            command.as_object_mut().unwrap().remove("id");
            assert!(serde_json::from_value::<Command>(command).is_err());
        }
    }

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
