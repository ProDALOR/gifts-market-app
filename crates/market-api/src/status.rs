//! Public market availability, independent of storage and synchronization protocols.
use serde::Serialize;
use utoipa::ToSchema;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MarketStatus {
    Available,
    Delayed,
    Unavailable,
}
impl From<&market_gifts::SyncStatus> for MarketStatus {
    fn from(status: &market_gifts::SyncStatus) -> Self {
        if !status.ready {
            Self::Unavailable
        } else if status.state == market_gifts::SyncState::Live {
            Self::Available
        } else {
            Self::Delayed
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn availability_preserves_usable_delayed_data_and_hides_operational_metadata() {
        use market_gifts::{SyncState, SyncStatus};
        for state in [
            SyncState::Starting,
            SyncState::Syncing,
            SyncState::Live,
            SyncState::Stale,
        ] {
            for ready in [false, true] {
                let source = SyncStatus {
                    ready,
                    state,
                    updated_at: Some("2026-11-19T00:00:00Z".parse().unwrap()),
                };
                let expected = if !ready {
                    "unavailable"
                } else if state == SyncState::Live {
                    "available"
                } else {
                    "delayed"
                };
                assert_eq!(
                    serde_json::to_value(MarketStatus::from(&source)).unwrap(),
                    expected
                );
            }
        }
    }
}
