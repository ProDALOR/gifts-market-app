//! Gift domain models plus the HTTP/WebSocket envelope. No public generic hub DTOs.
pub use crate::catalog::{
    GiftCatalog, GiftCollection, GiftCollectionCatalog, GiftMedia, GiftModel, GiftPattern,
};
pub use crate::orders::{
    OrderPageResponse, OrderResponse, OrderTargetResponse, UnsupportedTargetReason,
};
pub use market_gifts::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Subscribe {
        id: String,
        query: Box<GiftOrdersQuery>,
    },
    WatchCatalog {
        id: String,
        collection_ids: Vec<CollectionId>,
    },
    Unsubscribe {
        id: String,
    },
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame<'a> {
    Status {
        id: String,
        state: SubscriptionStatus,
    },
    Page {
        id: String,
        page: OrderPageResponse<'a>,
        /// Current replica freshness; authoritative over the cached page envelope.
        sync: SyncStatus,
        /// Opaque equality key for relevant data, independent of freshness.
        revision: String,
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
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    Connecting,
}

pub use crate::valuation::GiftValuationResponse;
