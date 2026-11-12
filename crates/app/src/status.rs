use crate::replication::ReplicaSnapshot;
use market_view::{Cursor, FeedState, MarketTimestamp, Record, RecordId, RecordKey};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApiCursor {
    pub generation: String,
    #[serde(with = "market_json::u64_string")]
    #[schema(value_type = String)]
    pub revision: u64,
}
impl From<Cursor> for ApiCursor {
    fn from(cursor: Cursor) -> Self {
        Self {
            generation: cursor.generation,
            revision: cursor.revision,
        }
    }
}
#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Live,
    Cached,
    Syncing,
}
#[derive(Clone, Serialize, ToSchema)]
pub struct ReplicaHealth {
    pub state: SyncState,
    #[schema(value_type=Option<String>,format=DateTime)]
    pub last_contact_at: Option<MarketTimestamp>,
}
#[derive(Clone, Serialize, ToSchema)]
pub struct ResponseMeta {
    pub cursor: Option<ApiCursor>,
    pub catalog_revision: Option<String>,
    #[schema(value_type=String,format=DateTime)]
    pub served_at: MarketTimestamp,
    /// Fixed across a paginated query; expiry is evaluated at this time.
    #[schema(value_type=String,format=DateTime)]
    pub as_of: MarketTimestamp,
    pub replica: ReplicaHealth,
}
impl ResponseMeta {
    pub fn new(snapshot: &ReplicaSnapshot, as_of: MarketTimestamp) -> Self {
        Self {
            cursor: snapshot
                .view
                .as_ref()
                .map(|view| view.cursor.clone().into()),
            catalog_revision: snapshot
                .view
                .as_ref()
                .map(|view| view.catalog.revision.clone()),
            served_at: MarketTimestamp::now(),
            as_of,
            replica: ReplicaHealth {
                state: if snapshot.view.is_none() {
                    SyncState::Syncing
                } else if snapshot.connected {
                    SyncState::Live
                } else {
                    SyncState::Cached
                },
                last_contact_at: snapshot.last_contact_at,
            },
        }
    }
}
#[derive(Clone, Serialize, ToSchema)]
pub struct StatusData {
    pub ready: bool,
    pub records: usize,
    pub feeds: usize,
    pub ready_feeds: usize,
}
impl StatusData {
    pub fn new(snapshot: &ReplicaSnapshot, max_stale: Duration) -> Self {
        let mut result = Self {
            ready: snapshot.can_serve(max_stale),
            records: 0,
            feeds: 0,
            ready_feeds: 0,
        };
        if let Some(view) = &snapshot.view {
            result.records = view.state.len();
            for feed in view.state.catalog().feeds() {
                result.feeds += 1;
                if matches!(view.state.get(&RecordKey {feed_id: feed.id.clone(), id: RecordId::Feed(feed.id.clone())}), Some(Record::Feed(status)) if status.state == FeedState::Ready)
                {
                    result.ready_feeds += 1;
                }
            }
        }
        result
    }
}

impl From<ApiCursor> for Cursor {
    fn from(v: ApiCursor) -> Self {
        Self {
            generation: v.generation,
            revision: v.revision,
        }
    }
}
