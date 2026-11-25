use crate::{LinkError, Result};
use market_gifts::*;
use market_gifts_hub::input::{GiftFilters, GiftOrderQuery};
use serde::{Deserialize, Serialize};
/// Navigation state only. The existing REST/WS order query contracts are unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct MarketSelection {
    pub filters: GiftFilters,
    #[serde(default)]
    pub source: OrderSource,
    #[serde(default = "exact")]
    pub bid_match: BidMatch,
    #[serde(default)]
    pub sort: Option<GiftSort>,
    #[serde(default)]
    pub period: Option<Period>,
    #[serde(default)]
    pub gift: Option<GiftSlug>,
}
fn exact() -> BidMatch {
    BidMatch::All
}
impl MarketSelection {
    pub fn new(filters: GiftFilters) -> Self {
        Self {
            filters,
            source: OrderSource::Asks,
            bid_match: exact(),
            sort: None,
            period: None,
            gift: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.filters.collection_id.is_none() && !self.filters.models.is_empty() {
            return Err(LinkError::Invalid("Model filters require a collection"));
        }
        if self.filters.models.len() + self.filters.backdrops.len() + self.filters.patterns.len()
            > 4096
        {
            return Err(LinkError::TooLarge);
        }
        self.filters
            .validate()
            .map_err(|_| LinkError::Invalid("Invalid market filter range"))?;
        if let Some(period) = &self.period {
            period
                .validate()
                .map_err(|_| LinkError::Invalid("Invalid history period"))?;
        }
        if self.sort == Some(GiftSort::Identity) {
            return Err(LinkError::Invalid("Unsupported market view sort"));
        }
        Ok(())
    }
    pub fn order_query(&self) -> GiftOrderQuery {
        let mut query = GiftOrderQuery::new(self.filters.clone());
        query.source = self.source;
        query.bid_match = if self.source == OrderSource::Bids {
            self.bid_match
        } else {
            BidMatch::Any
        };
        query.sort = self.sort;
        query.period = if self.source == OrderSource::History {
            self.period.clone()
        } else {
            None
        };
        query
    }
}
