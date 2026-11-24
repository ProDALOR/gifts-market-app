//! Admission policy for gift queries; wire translation lives in market-gifts-hub.
use crate::error::{DataError, Result};
use market_gifts::{GiftFilters, GiftOrderQuery};
use market_hub_protocol::wire as w;
fn admit(filters: &GiftFilters, limit: u32) -> Result<()> {
    if !(1..=market_gifts::ORDER_LIMIT).contains(&limit) {
        return Err(DataError::invalid(format!(
            "row count must be 1..={}",
            market_gifts::ORDER_LIMIT
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

/// One range compiler for finite reads and watches.
pub(crate) fn compile(
    query: GiftOrderQuery,
) -> Result<(w::ReadRequest, market_replica::RangeRequest)> {
    use market_gifts::ReadView;
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
    let options = market_replica::ReadOptions {
        sort: Some(match query.sort() {
            market_gifts::GiftSort::Identity => market_replica::Sort::Identity,
            market_gifts::GiftSort::PriceAsc => market_replica::Sort::PriceAsc,
            market_gifts::GiftSort::PriceDesc => market_replica::Sort::PriceDesc,
            market_gifts::GiftSort::TimeAsc => market_replica::Sort::TimeAsc,
            market_gifts::GiftSort::TimeDesc => market_replica::Sort::TimeDesc,
            market_gifts::GiftSort::NumberAsc => market_replica::Sort::NumberAsc,
            market_gifts::GiftSort::NumberDesc => market_replica::Sort::NumberDesc,
        }),
        attribute_match: match (query.source, query.bid_match) {
            (market_gifts::OrderSource::Bids, market_gifts::BidMatch::Any) => {
                market_replica::AttributeMatch::Overlap
            }
            (market_gifts::OrderSource::Bids, market_gifts::BidMatch::All) => {
                market_replica::AttributeMatch::Exact
            }
            _ => market_replica::AttributeMatch::Value,
        },
    }
    .normalized_for(&request);
    Ok((
        request,
        market_replica::RangeRequest {
            options,
            range: range(&query.range)?,
            limit: query.limit,
            snapshot,
        },
    ))
}
fn range(range: &market_gifts::OrderRange) -> Result<market_replica::KeyRange> {
    let decode = |p: &market_gifts::OrderKey| market_replica::ReadPosition::decode(p.as_str());
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_gifts::{BidMatch, OrderSource};
    use market_replica::{AttributeMatch, ReadOptions};
    #[test]
    fn all_local_sorts_use_native_options_and_leave_the_wire_contract_unchanged() {
        use market_gifts::GiftSort;
        use market_replica::Sort;
        let mut query = GiftOrderQuery::new(GiftFilters::new("gift".into(), "GRAM".into()));
        let mut baseline = None;
        for (gift, local) in [
            (GiftSort::Identity, Sort::Identity),
            (GiftSort::PriceAsc, Sort::PriceAsc),
            (GiftSort::PriceDesc, Sort::PriceDesc),
            (GiftSort::TimeAsc, Sort::TimeAsc),
            (GiftSort::TimeDesc, Sort::TimeDesc),
            (GiftSort::NumberAsc, Sort::NumberAsc),
            (GiftSort::NumberDesc, Sort::NumberDesc),
        ] {
            query.sort = Some(gift);
            let (request, read) = compile(query.clone()).unwrap();
            assert_eq!(request.sort, w::ReadSort::Identity as i32);
            assert_eq!(read.options.sort, Some(local));
            if let Some(baseline) = &baseline {
                assert_eq!(&request, baseline);
            } else {
                baseline = Some(request);
            }
        }
    }
    #[test]
    fn bid_matching_changes_local_options_without_changing_wire_requests() {
        let mut query = GiftOrderQuery::new(GiftFilters::new("gift".into(), "GRAM".into()));
        query.source = OrderSource::Bids;
        query.filters.models = vec!["gold".into(), "silver".into()];
        let (any_request, any_read) = compile(query.clone()).unwrap();
        query.bid_match = BidMatch::All;
        let (all_request, all_read) = compile(query.clone()).unwrap();
        assert_eq!(any_request, all_request);
        assert_eq!(any_read.options.attribute_match, AttributeMatch::Overlap);
        assert_eq!(all_read.options.attribute_match, AttributeMatch::Exact);
        query.filters.models.clear();
        for (mode, options) in [
            (
                BidMatch::Any,
                ReadOptions {
                    sort: Some(market_replica::Sort::PriceDesc),
                    ..Default::default()
                },
            ),
            (
                BidMatch::All,
                ReadOptions {
                    attribute_match: AttributeMatch::Exact,
                    sort: Some(market_replica::Sort::PriceDesc),
                },
            ),
        ] {
            query.bid_match = mode;
            assert_eq!(compile(query.clone()).unwrap().1.options, options);
        }
    }
}
