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
pub struct WindowEntry<T> {
    pub order: T,
    pub position: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OrderWindow {
    pub data: Vec<WindowEntry<crate::GiftOrder>>,
    pub anchor: Option<String>,
    pub has_before: bool,
    pub has_after: bool,
    pub cursor: String,
}

#[derive(Debug, Clone)]
pub struct SnapshotWindow {
    pub window: OrderWindow,
    pub snapshot: String,
    pub sync: SyncStatus,
    pub revision: String,
}
