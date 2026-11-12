use market_view::{Cursor, MarketTimestamp};
use serde::{Deserialize, Serialize};
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
pub struct ResponseMeta {
    pub cursor: Option<ApiCursor>,
    pub catalog_revision: Option<String>,
    #[schema(value_type=String,format=DateTime)]
    pub served_at: MarketTimestamp,
    /// Fixed across a paginated query; expiry is evaluated at this time.
    #[schema(value_type=String,format=DateTime)]
    pub as_of: MarketTimestamp,
}
#[derive(Clone, Serialize, ToSchema)]
pub struct StatusData {
    pub ready: bool,
    pub records: usize,
    pub feeds: usize,
    pub ready_feeds: usize,
}
impl From<ApiCursor> for Cursor {
    fn from(v: ApiCursor) -> Self {
        Self {
            generation: v.generation,
            revision: v.revision,
        }
    }
}
