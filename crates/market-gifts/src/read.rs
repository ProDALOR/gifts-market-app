//! Paging, consistency and replica freshness exposed by gift reads.
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ReadConsistency {
    #[default]
    Snapshot,
    Live,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Page<T> {
    pub data: Vec<T>,
    pub next_page_token: String,
    pub cursor: String,
    pub sync: SyncStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum SyncState {
    Starting,
    Syncing,
    Live,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct SyncStatus {
    pub ready: bool,
    pub state: SyncState,
    #[cfg_attr(feature = "schema", schema(value_type = Option<String>, format = DateTime))]
    pub updated_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderedEntry<T> {
    pub order: T,
    pub position: String,
    pub sort_key: String,
}

/// Inclusive, fixed ordering boundary. The position is query-bound; sort_key is comparable ASCII.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct OrderBoundary {
    pub position: String,
    pub sort_key: String,
}
/// Null start/end means the open head/tail of the query, respectively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct OrderRange {
    #[cfg_attr(feature = "schema", schema(required = true))]
    pub start: Option<OrderBoundary>,
    #[cfg_attr(feature = "schema", schema(required = true))]
    pub end: Option<OrderBoundary>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OrderResult {
    pub data: Vec<OrderedEntry<crate::GiftOrder>>,
    pub range: OrderRange,
}
#[derive(Debug, Clone)]
pub struct OrderRead {
    pub result: OrderResult,
    pub snapshot: Option<String>,
    pub sync: SyncStatus,
    pub revision: String,
}
