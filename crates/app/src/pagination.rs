use crate::{
    error::{ApiError, ErrorCode, Result},
    replication::ReplicaView,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use market_query::BookCursor;
use market_view::{MarketTimestamp, RecordKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub fn limit(value: Option<usize>) -> Result<usize> {
    let value = value.unwrap_or(50);
    if !(1..=200).contains(&value) {
        return Err(ApiError::field("limit", "must be between 1 and 200"));
    }
    Ok(value)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Position {
    Catalog(usize),
    Item(RecordKey),
    Order(BookCursor),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Token {
    pub snapshot: String,
    pub query: String,
    pub as_of: MarketTimestamp,
    pub position: Position,
}
impl Token {
    pub fn decode(raw: &str) -> Result<Self> {
        if raw.len() > 4096 {
            return Err(ApiError::new(
                ErrorCode::InvalidCursor,
                "cursor is too long",
            ));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(raw)
            .map_err(|_| ApiError::new(ErrorCode::InvalidCursor, "invalid cursor"))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| ApiError::new(ErrorCode::InvalidCursor, "invalid cursor"))
    }
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("cursor serializes"))
    }
}
pub struct SavedView {
    pub id: String,
    pub view: Arc<ReplicaView>,
    deadline: Instant,
    bytes: usize,
}
pub struct Views {
    entries: VecDeque<Arc<SavedView>>,
    bytes: usize,
    pub ttl: Duration,
    pub max_views: usize,
    pub max_bytes: usize,
}
impl Default for Views {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            ttl: Duration::from_secs(60),
            max_views: 64,
            max_bytes: 512 * 1024 * 1024,
        }
    }
}
impl Views {
    fn prune(&mut self, generation: &str) {
        self.entries.retain(|entry| {
            entry.deadline > Instant::now() && entry.view.cursor.generation == generation
        });
        self.bytes = self.entries.iter().map(|entry| entry.bytes).sum();
    }
    pub fn get(&mut self, id: &str, generation: &str) -> Result<Arc<SavedView>> {
        self.prune(generation);
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCode::CursorExpired,
                    "snapshot expired or belongs to another API node; reload the query",
                )
            })
    }
    pub fn save(&mut self, view: Arc<ReplicaView>) -> Result<String> {
        self.prune(&view.cursor.generation);
        if let Some(existing) = self
            .entries
            .iter()
            .find(|entry| entry.view.cursor == view.cursor)
        {
            return Ok(existing.id.clone());
        }
        let bytes = view.record_bytes();
        if bytes > self.max_bytes {
            return Err(ApiError::new(
                ErrorCode::CapacityExceeded,
                "snapshot exceeds retained-view budget",
            ));
        }
        while self.entries.len() >= self.max_views
            || self.bytes.saturating_add(bytes) > self.max_bytes
        {
            if let Some(entry) = self.entries.pop_front() {
                self.bytes -= entry.bytes;
            } else {
                break;
            }
        }
        let id = Uuid::new_v4().simple().to_string();
        self.entries.push_back(Arc::new(SavedView {
            id: id.clone(),
            view,
            deadline: Instant::now() + self.ttl,
            bytes,
        }));
        self.bytes += bytes;
        Ok(id)
    }
}
