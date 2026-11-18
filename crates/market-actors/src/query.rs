//! Admission policy for gift queries; wire translation lives in market-gifts-hub.
use crate::error::{DataError, Result};
use market_gifts::{GiftFilters, GiftHistoryQuery, GiftOrdersQuery};
use market_hub_protocol::wire as w;
use tonic::Status;
fn admit(filters: &GiftFilters, limit: u32) -> Result<()> {
    if !(1..=200).contains(&limit) {
        return Err(DataError::invalid("limit must be 1..200"));
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

/// Compile live and frozen windows through the same domain validation/admission path.
pub(crate) fn window(
    query: market_gifts::GiftOrderWindowQuery,
    mode: w::ReadMode,
    snapshot: Option<String>,
) -> Result<(w::ReadRequest, market_replica::WindowRequest)> {
    query
        .validate()
        .map_err(|e| DataError::invalid(e.to_string()))?;
    let window = market_replica::WindowRequest {
        anchor: query.anchor,
        before: query.before,
        after: query.after,
        snapshot,
    };
    let mut read = GiftOrdersQuery::new(query.filters);
    read.side = query.side;
    read.sort = query.sort;
    read.limit = 200;
    Ok((orders(read, mode)?, window))
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
