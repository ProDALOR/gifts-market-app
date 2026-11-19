//! Paging, consistency and replica freshness exposed by gift reads.
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

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

/// Opaque ordering position, reusable across filters with the same sort/source.
/// Compare in ordinary ASCII order. Price ranks use the supplied query's currency units.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", schema(value_type = String, pattern="^[0-9A-Za-z_~]{2,8192}$"))]
pub struct OrderKey(String);
impl OrderKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for OrderKey {
    type Error = crate::InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() < 2
            || value.len() > 8192
            || value.len() % 4 == 1
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'~')
        {
            return Err(crate::InvalidValue("invalid ordering key"));
        }
        Ok(Self(value))
    }
}
impl From<OrderKey> for String {
    fn from(value: OrderKey) -> Self {
        value.0
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", schema(value_type = String, format="uuid"))]
pub struct ReadSnapshotId(String);
impl ReadSnapshotId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for ReadSnapshotId {
    type Error = crate::InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 36
            || value.bytes().enumerate().any(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b != b'-'
                } else {
                    !b.is_ascii_hexdigit()
                }
            })
        {
            return Err(crate::InvalidValue("invalid snapshot ID"));
        }
        Ok(Self(value))
    }
}
impl From<ReadSnapshotId> for String {
    fn from(value: ReadSnapshotId) -> Self {
        value.0
    }
}
/// Inclusive fixed bounds. Ordering keys may encode the point immediately before/after a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct OrderRange {
    #[cfg_attr(feature = "schema", schema(required = true))]
    pub start: Option<OrderKey>,
    #[cfg_attr(feature = "schema", schema(required = true))]
    pub end: Option<OrderKey>,
}
