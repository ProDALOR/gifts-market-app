//! Admission policy for gift queries; wire translation lives in market-gifts-hub.
use crate::error::{DataError, Result};
use market_gifts::{GiftFilters, GiftOrderQuery};
use market_hub_protocol::wire as w;
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

/// One query compiler for finite HTTP/WS reads and atomic watch initialization.
pub(crate) fn selection(
    query: GiftOrderQuery,
) -> Result<(w::ReadRequest, market_replica::SliceRequest)> {
    use market_gifts::{Navigation as N, ReadView};
    use market_replica::{Navigation, ReadPosition};
    admit(&query.filters, query.limit)?;
    let mut request =
        market_gifts_hub::query::compile(&query).map_err(|e| DataError::invalid(e.to_string()))?;
    let snapshot = match &query.view {
        ReadView::Latest {} => None,
        ReadView::Snapshot { id } => {
            request.mode = w::ReadMode::Snapshot as i32;
            id.as_ref().map(|id| id.as_str().parse()).transpose()?
        }
    };
    let decode = |key: market_gifts::OrderKey| ReadPosition::decode(key.as_str());
    let navigation = match query.navigation {
        N::Head {} => Navigation::Head,
        N::After { key } => Navigation::After(decode(key)?),
        N::Before { key } => Navigation::Before(decode(key)?),
        N::Around { key } => Navigation::Around(decode(key)?),
    };
    Ok((
        request,
        market_replica::SliceRequest {
            navigation,
            limit: query.limit,
            snapshot,
        },
    ))
}
pub(crate) fn range(
    range: Option<&market_gifts::OrderRange>,
) -> Result<Option<market_replica::KeyRange>> {
    range
        .map(|range| {
            let decode =
                |p: &market_gifts::OrderKey| market_replica::ReadPosition::decode(p.as_str());
            let range = market_replica::KeyRange {
                start: range.start.as_ref().map(decode).transpose()?,
                end: range.end.as_ref().map(decode).transpose()?,
            };
            if range
                .start
                .as_ref()
                .zip(range.end.as_ref())
                .is_some_and(|(a, b)| a > b)
            {
                return Err(DataError::invalid("inverted order range"));
            }
            Ok(range)
        })
        .transpose()
}
