//! Admission policy for gift queries; wire translation lives in market-gifts-hub.
use crate::error::{DataError, Result};
use market_gifts::{GiftFilters, GiftHistoryQuery, GiftOrdersQuery};
use market_hub_protocol::wire as w;
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
