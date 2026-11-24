//! Replica envelopes and actor errors; gift interpretation belongs to market-gifts-hub.
use crate::error::{DataError, Result};
use market_gifts::*;
use market_hub_protocol::wire as w;
pub(crate) fn invalid(value: impl std::fmt::Display) -> DataError {
    DataError::unavailable(value.to_string())
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
pub(crate) fn order_result(result: market_replica::ReadBatch) -> Result<crate::OrderRead> {
    let data: Vec<_> = result
        .entries
        .into_iter()
        .map(|entry| {
            Ok(crate::OrderedEntry {
                order: match entry.entity.entity.as_ref() {
                    Some(w::entity::Entity::Order(order)) => {
                        market_gifts_hub::order_ref(order).map_err(invalid)?
                    }
                    _ => return Err(invalid("expected a gift order")),
                },
                key: entry.key,
            })
        })
        .collect::<Result<_>>()?;
    let mut identities = std::collections::HashSet::new();
    if data
        .iter()
        .any(|entry| !identities.insert((&entry.order.book.venue_id, &entry.order.id)))
    {
        return Err(DataError::unavailable(
            "Duplicate order identities in query result",
        ));
    }
    Ok(crate::OrderRead {
        result: crate::OrderResult {
            data,
            range: result.range,
        },
        snapshot: result
            .snapshot
            .map(|id| id.to_string().try_into().map_err(invalid))
            .transpose()?,
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

/// Projection stays inside the admitted blocking read, including its cancellation/slot lifetime.
pub(crate) fn projected_read(
    batch: market_replica::ReadBatch,
) -> std::result::Result<crate::OrderSnapshot, tonic::Status> {
    let version = batch.revision;
    let observed = batch.observed;
    let read =
        order_result(batch).map_err(|error| tonic::Status::unavailable(error.to_string()))?;
    Ok(crate::OrderSnapshot {
        read,
        version,
        observed,
    })
}
