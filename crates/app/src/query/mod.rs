//! All public transports use these local queries; no handler calls core.
pub mod models;
mod read;
use crate::{
    error::{ApiError, ErrorCode, Result},
    pagination::{Position, Token, Views},
    replication::{ReplicaHandle, ReplicaSnapshot, ReplicaView},
    status::{ResponseMeta, StatusData},
};
use market_view::MarketTimestamp;
pub use models::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub struct ApiState {
    pub replica: ReplicaHandle,
    pub max_stale: Duration,
    pub max_scan: usize,
    pub(crate) slots: Arc<Semaphore>,
    pub(crate) streams: Arc<Semaphore>,
    views: Arc<Mutex<Views>>,
    pub(crate) snapshot_slots: Arc<Semaphore>,
    pub(crate) snapshot_bytes: Arc<Semaphore>,
    book_cache: Arc<Mutex<HashMap<String, CachedBook>>>,
}
struct CachedBook {
    at: Instant,
    cursor: market_view::Cursor,
    connected: bool,
    response: Arc<BookResponse>,
    json: Arc<serde_json::value::RawValue>,
}
pub struct QueryView {
    pub snapshot: ReplicaSnapshot,
    pub view: Arc<ReplicaView>,
    pub as_of: MarketTimestamp,
    pub position: Option<Position>,
    hash: String,
}
impl QueryView {
    pub fn meta(&self) -> ResponseMeta {
        ResponseMeta::new(&self.snapshot, self.as_of)
    }
}
impl ApiState {
    pub fn new(
        replica: ReplicaHandle,
        max_stale: Duration,
        max_concurrent: usize,
        max_scan: usize,
    ) -> Self {
        assert!(max_concurrent > 0 && max_scan > 0);
        Self {
            replica,
            max_stale,
            max_scan,
            slots: Arc::new(Semaphore::new(max_concurrent)),
            streams: Arc::new(Semaphore::new(128)),
            views: Arc::new(Mutex::new(Views::default())),
            snapshot_slots: Arc::new(Semaphore::new(8)),
            snapshot_bytes: Arc::new(Semaphore::new(512 * 1024 * 1024)),
            book_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn limits(mut self, streams: usize, snapshots: usize, bytes: usize, ttl: Duration) -> Self {
        assert!(streams > 0 && snapshots > 0 && bytes > 0 && !ttl.is_zero());
        self.streams = Arc::new(Semaphore::new(streams));
        self.snapshot_slots = Arc::new(Semaphore::new(snapshots));
        self.snapshot_bytes = Arc::new(Semaphore::new(bytes));
        let mut views = self.views.lock().expect("view cache poisoned");
        views.max_views = snapshots;
        views.max_bytes = bytes;
        views.ttl = ttl;
        drop(views);
        self
    }
    pub fn status(&self) -> StatusResponse {
        let snapshot = self.replica.snapshot();
        Response {
            data: StatusData::new(&snapshot, self.max_stale),
            page: None,
            meta: ResponseMeta::new(&snapshot, MarketTimestamp::now()),
            included: None,
        }
    }
    pub fn stream_slot(&self) -> Result<OwnedSemaphorePermit> {
        self.streams.clone().try_acquire_owned().map_err(|_| {
            ApiError::new(ErrorCode::CapacityExceeded, "API stream capacity exhausted")
        })
    }
    pub fn current(&self) -> Result<ReplicaSnapshot> {
        let snapshot = self.replica.snapshot();
        if !snapshot.can_serve(self.max_stale) {
            return Err(ApiError::unavailable());
        }
        Ok(snapshot)
    }
    fn context<T: Serialize>(
        &self,
        query: &T,
        cursor: Option<&str>,
    ) -> Result<(QueryView, OwnedSemaphorePermit)> {
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            ApiError::new(ErrorCode::CapacityExceeded, "API query capacity exhausted")
        })?;
        let mut snapshot = self.current()?;
        let mut view = snapshot.view.as_ref().expect("servable snapshot").clone();
        let hash = fingerprint(query);
        let (as_of, position) = if let Some(cursor) = cursor {
            let token = Token::decode(cursor)?;
            if token.query != hash {
                return Err(ApiError::new(
                    ErrorCode::CursorMismatch,
                    "cursor does not match query filters or sorting",
                ));
            }
            let saved = self
                .views
                .lock()
                .expect("view cache poisoned")
                .get(&token.snapshot, &view.cursor.generation)?;
            view = saved.view.clone();
            (token.as_of, Some(token.position))
        } else {
            (MarketTimestamp::now(), None)
        };
        snapshot.view = Some(view.clone());
        Ok((
            QueryView {
                snapshot,
                view,
                as_of,
                position,
                hash,
            },
            permit,
        ))
    }
    fn cursor(&self, context: &QueryView, position: Position) -> Result<String> {
        let snapshot = self
            .views
            .lock()
            .expect("view cache poisoned")
            .save(context.view.clone())?;
        Ok(Token {
            snapshot,
            query: context.hash.clone(),
            as_of: context.as_of,
            position,
        }
        .encode())
    }
    /// Shared computation and JSON encoding for identical bounded book subscriptions.
    pub fn cached_book(
        &self,
        query: &BookQuery,
    ) -> Result<(Arc<BookResponse>, Arc<serde_json::value::RawValue>)> {
        let snapshot = self.current()?;
        let cursor = &snapshot.view.as_ref().expect("servable snapshot").cursor;
        let key = fingerprint(query);
        let mut cache = self.book_cache.lock().expect("book cache poisoned");
        if let Some(entry) = cache.get(&key)
            && entry.cursor == *cursor
            && entry.connected == snapshot.connected
            && entry.at.elapsed() < Duration::from_secs(1)
        {
            return Ok((entry.response.clone(), entry.json.clone()));
        }
        let response = Arc::new(self.orderbook(query)?);
        let json: Arc<serde_json::value::RawValue> = serde_json::value::RawValue::from_string(
            serde_json::to_string(&*response).expect("book serializes"),
        )
        .expect("book JSON")
        .into();
        while (!cache.contains_key(&key) && cache.len() >= 256)
            || cache
                .values()
                .map(|v| v.json.get().len() * 2)
                .sum::<usize>()
                + json.get().len() * 2
                > 32 * 1024 * 1024
        {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, value)| value.at)
                .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest);
            } else {
                break;
            }
        }
        cache.insert(
            key,
            CachedBook {
                at: Instant::now(),
                cursor: response
                    .meta
                    .cursor
                    .clone()
                    .expect("book has cursor")
                    .into(),
                connected: matches!(response.meta.replica.state, crate::status::SyncState::Live),
                response: response.clone(),
                json: json.clone(),
            },
        );
        Ok((response, json))
    }
}
pub fn fingerprint<T: Serialize>(value: &T) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("query serializes"))
    )
}
