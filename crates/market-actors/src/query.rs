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
pub(crate) fn valuation(request: &market_gifts::valuation::ValuationRequest) -> Result<()> {
    if request.positions.len() > 1000 {
        return Err(DataError::invalid(
            "at most 1000 model positions are allowed",
        ));
    }
    request
        .validate()
        .map_err(|error| DataError::invalid(error.to_string()))?;
    if request
        .positions
        .iter()
        .map(|position| u64::from(position.quantity))
        .sum::<u64>()
        > 10_000
    {
        return Err(DataError::invalid("portfolio quantity must be 1..10000"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DataErrorCode;
    use market_gifts::{
        GiftNumberRange,
        valuation::{ModelLookup, ModelPosition, ValuationRequest},
    };
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
            min: Some(10),
            max: Some(1),
        });
        assert_eq!(
            orders(reversed, w::ReadMode::Live).unwrap_err().code(),
            DataErrorCode::InvalidQuery
        );
        let position = ModelPosition {
            model: ModelLookup::Catalog {
                collection: "collection".into(),
                model: "model".into(),
            },
            quantity: 1,
        };
        let mut request = ValuationRequest {
            strategy: Default::default(),
            positions: vec![position.clone(); 1001],
            currency_id: "GRAM".into(),
        };
        assert!(
            request.validate().is_ok(),
            "position count is service policy"
        );
        assert_eq!(
            valuation(&request).unwrap_err().code(),
            DataErrorCode::InvalidQuery
        );
        request.positions = vec![ModelPosition {
            quantity: 10001,
            ..position.clone()
        }];
        assert!(request.validate().is_ok(), "work limits are service policy");
        assert_eq!(
            valuation(&request).unwrap_err().code(),
            DataErrorCode::InvalidQuery
        );
        request.positions = vec![position];
        assert!(valuation(&request).is_ok());
    }
}
