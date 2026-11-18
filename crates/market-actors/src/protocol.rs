//! Replica envelopes and actor errors; gift interpretation belongs to market-gifts-hub.
use crate::error::{DataError, Result};
use market_gifts::*;
use market_gifts_hub::order;
use market_hub_protocol::wire as w;
pub(crate) fn invalid(value: impl std::fmt::Display) -> DataError {
    DataError::unavailable(value.to_string())
}
fn required<T>(value: Option<T>) -> Result<T> {
    value.ok_or_else(|| invalid("missing gift market field"))
}
fn time(value: i64) -> Result<jiff::Timestamp> {
    jiff::Timestamp::from_millisecond(value).map_err(invalid)
}
pub(crate) fn book(value: w::BookKey) -> Result<GiftBook> {
    market_gifts_hub::book(value).map_err(invalid)
}
pub(crate) fn sync(value: market_replica::SyncStatus) -> Result<SyncStatus> {
    Ok(SyncStatus {
        ready: value.ready,
        state: match value.state {
            market_replica::SyncState::Starting => SyncState::Starting,
            market_replica::SyncState::Syncing => SyncState::Syncing,
            market_replica::SyncState::Live => SyncState::Live,
            market_replica::SyncState::Stale => SyncState::Stale,
        },
        updated_at: value.updated_at.map(time).transpose()?,
    })
}
pub(crate) fn orders(result: market_replica::ReadResult) -> Result<Page<GiftOrder>> {
    Ok(Page {
        sync: sync(result.sync)?,
        cursor: result.page.cursor,
        next_page_token: result.page.next_page_token,
        data: result
            .page
            .entities
            .into_iter()
            .map(|entity| match required(entity.entity)? {
                w::entity::Entity::Order(value) => order(value).map_err(invalid),
                _ => Err(invalid("expected a gift order")),
            })
            .collect::<Result<_>>()?,
    })
}

/// Gift-specific eligibility belongs outside the generic replica's bounded scan.
pub(crate) fn include_order(entity: &w::Entity) -> std::result::Result<bool, tonic::Status> {
    match entity.entity.as_ref() {
        Some(w::entity::Entity::Order(value)) => market_gifts_hub::is_exhausted(value)
            .map(|exhausted| !exhausted)
            .map_err(|e| tonic::Status::unavailable(e.to_string())),
        _ => Err(tonic::Status::unavailable("expected a gift order")),
    }
}
