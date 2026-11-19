//! Admission policy for gift queries; wire translation lives in market-gifts-hub.
use crate::error::{DataError, Result};
use market_gifts::{GiftFilters, GiftHistoryQuery, GiftOrdersQuery};
use market_hub_protocol::wire as w;
use tonic::Status;
fn admit(filters: &GiftFilters, limit: u32) -> Result<()> {
    if !(1..=market_replica::READ_LIMITS.max_rows).contains(&limit) {
        return Err(DataError::invalid(format!(
            "row count must be 1..={}",
            market_replica::READ_LIMITS.max_rows
        )));
    }
    if filters.models.len() + filters.backdrops.len() + filters.patterns.len() > 500 {
        return Err(DataError::invalid("at most 500 gift trait selections"));
    }
    Ok(())
}
pub(crate) fn orders(query: GiftOrdersQuery, mode: w::ReadMode) -> Result<w::ReadRequest> {
    admit(&query.filters, query.limit)?;
    market_gifts_hub::query::orders(query, mode).map_err(|e| DataError::invalid(e.to_string()))
}
pub(crate) fn history(query: GiftHistoryQuery) -> Result<w::ReadRequest> {
    admit(&query.filters, query.limit)?;
    market_gifts_hub::query::history(query).map_err(|e| DataError::invalid(e.to_string()))
}

/// Validate collection identity against the same pinned view as the order read.
pub(crate) fn validate_order_collection(
    reader: &market_replica::ReplicaReader,
    request: &w::ReadRequest,
) -> std::result::Result<(), tonic::Status> {
    if let Some(w::selection::Source::Scope(scope)) =
        request.selection.as_ref().and_then(|s| s.source.as_ref())
        && let Some(id) = &scope.item_type_id
    {
        let items = market_replica::catalog_read::entities(
            reader,
            w::EntityKind::ItemType,
            Some(std::slice::from_ref(id)),
        )?;
        if !items.iter().any(|item| matches!(item, w::entity::Entity::ItemType(item) if market_gifts_hub::is_gift(item))) {
            return Err(tonic::Status::invalid_argument("unknown gift collection"));
        }
    }
    Ok(())
}

pub(crate) fn validate_read(request: &w::ReadRequest) -> std::result::Result<(), Status> {
    let selection = request
        .selection
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("select an orderbook scope or order identities"))?;
    if selection.source.is_some() || selection.executed_at.is_some() {
        return Ok(());
    }
    if (!selection.kinds.is_empty() || !selection.keys.is_empty())
        && selection
            .kinds
            .iter()
            .all(|kind| *kind == w::EntityKind::Order as i32)
        && selection
            .keys
            .iter()
            .all(|key| key.kind == w::EntityKind::Order as i32)
    {
        Ok(())
    } else {
        Err(Status::invalid_argument(
            "orderbook queries must select only orders",
        ))
    }
}

/// One query compiler for finite HTTP/WS reads and atomic watch initialization.
pub(crate) fn selection(
    query: market_gifts::GiftOrderQuery,
    mode: w::ReadMode,
    snapshot: Option<String>,
) -> Result<(w::ReadRequest, market_replica::WindowRequest)> {
    use market_gifts::{OrderSource, ReadDirection};
    if query.source != OrderSource::History && query.period.is_some() {
        return Err(DataError::invalid("period is only valid for history"));
    }
    if query
        .position
        .as_ref()
        .is_some_and(|s| s.is_empty() || s.len() > market_replica::READ_LIMITS.max_position_bytes)
    {
        return Err(DataError::invalid("invalid order position"));
    }
    if mode == w::ReadMode::Live && snapshot.is_some() {
        return Err(DataError::invalid(
            "latest reads do not use snapshot leases",
        ));
    }
    admit(&query.filters, query.limit)?;
    let before = match query.direction {
        ReadDirection::Forward => 0,
        ReadDirection::Backward => query.limit - 1,
        ReadDirection::Around => query.limit / 3,
    };
    let window = market_replica::WindowRequest {
        fill_edges: query.direction == ReadDirection::Around,
        anchor: query.position,
        before,
        after: query.limit - before - 1,
        snapshot,
    };
    let request = if query.source == OrderSource::History {
        let mut history = GiftHistoryQuery::new(query.filters);
        history.sort = query.sort;
        history.period = query.period;
        history.limit = query.limit;
        let mut request = self::history(history)?;
        request.mode = mode as i32;
        request
    } else {
        let mut orders = GiftOrdersQuery::new(query.filters);
        orders.side = if query.source == OrderSource::Bids {
            market_gifts::GiftSide::Bid
        } else {
            market_gifts::GiftSide::Ask
        };
        orders.sort = query.sort;
        orders.limit = query.limit;
        self::orders(orders, mode)?
    };
    Ok((request, window))
}

pub(crate) fn range(
    range: Option<&market_gifts::OrderRange>,
) -> Result<Option<market_replica::WindowRange>> {
    range
        .map(|range| {
            for boundary in range.start.iter().chain(range.end.iter()) {
                if market_replica::position_sort_key(&boundary.position).map_err(DataError::from)?
                    != boundary.sort_key
                {
                    return Err(DataError::invalid(
                        "range ordering key does not match position",
                    ));
                }
            }
            if let (Some(start), Some(end)) = (&range.start, &range.end)
                && start.sort_key > end.sort_key
            {
                return Err(DataError::invalid("inverted order range"));
            }
            Ok(market_replica::WindowRange {
                start: range.start.as_ref().map(|b| b.position.clone()),
                end: range.end.as_ref().map(|b| b.position.clone()),
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DataErrorCode;
    use market_gifts::{GiftNumber, GiftNumberRange};
    #[test]
    fn query_admission_keeps_service_limits_and_classifies_domain_errors() {
        let base = GiftOrdersQuery::new(GiftFilters::new("collection".into(), "GRAM".into()));
        for limit in [0, 201, u32::MAX] {
            let mut query = base.clone();
            query.limit = limit;
            assert_eq!(
                orders(query, w::ReadMode::Live).unwrap_err().code(),
                DataErrorCode::InvalidQuery
            );
        }
        let mut oversized = base.clone();
        oversized.filters.models = vec!["model".into(); 501];
        assert_eq!(
            orders(oversized, w::ReadMode::Live).unwrap_err().code(),
            DataErrorCode::InvalidQuery
        );
        let mut reversed = base;
        reversed.filters.number = Some(GiftNumberRange {
            min: Some(GiftNumber::new(10).unwrap()),
            max: Some(GiftNumber::new(1).unwrap()),
        });
        assert_eq!(
            orders(reversed, w::ReadMode::Live).unwrap_err().code(),
            DataErrorCode::InvalidQuery
        );
    }
}
