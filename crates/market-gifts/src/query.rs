//! Gift query contracts and reusable range and period invariants.
use crate::{
    Amount, BackdropId, CollectionId, CurrencyId, GiftNumber, InvalidValue, ModelId, PatternId,
    VenueId,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

pub const ORDER_LIMIT: u32 = 100;

fn default_limit() -> u32 {
    ORDER_LIMIT
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftSort {
    Identity,
    #[default]
    PriceAsc,
    PriceDesc,
    TimeAsc,
    TimeDesc,
    NumberAsc,
    NumberDesc,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PriceRange {
    pub min: Option<Amount>,
    pub max: Option<Amount>,
}
impl PriceRange {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if self
            .min
            .as_ref()
            .zip(self.max.as_ref())
            .is_some_and(|(min, max)| min.decimal() > max.decimal())
        {
            return Err(InvalidValue("invalid gift price range"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftNumberRange {
    pub min: Option<GiftNumber>,
    pub max: Option<GiftNumber>,
}
impl GiftNumberRange {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if self.min.zip(self.max).is_some_and(|(min, max)| min > max) {
            return Err(InvalidValue("invalid gift number range"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftFilters {
    pub collection_id: CollectionId,
    pub currency_id: CurrencyId,
    pub venue_id: Option<VenueId>,
    #[serde(default)]
    pub models: Vec<ModelId>,
    #[serde(default)]
    pub backdrops: Vec<BackdropId>,
    #[serde(default)]
    pub patterns: Vec<PatternId>,
    pub number: Option<GiftNumberRange>,
    pub price: Option<PriceRange>,
}
impl GiftFilters {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if let Some(range) = &self.number {
            range.validate()?;
        }
        if let Some(range) = &self.price {
            range.validate()?;
        }
        Ok(())
    }
    pub fn new(collection_id: CollectionId, currency_id: CurrencyId) -> Self {
        Self {
            collection_id,
            currency_id,
            venue_id: None,
            models: vec![],
            backdrops: vec![],
            patterns: vec![],
            number: None,
            price: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Period {
    #[cfg_attr(feature = "schema", schema(value_type = Option<String>, format = DateTime))]
    pub from: Option<Timestamp>,
    #[cfg_attr(feature = "schema", schema(value_type = Option<String>, format = DateTime))]
    pub until: Option<Timestamp>,
}
impl Period {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if self
            .from
            .zip(self.until)
            .is_some_and(|(from, until)| from > until)
        {
            return Err(InvalidValue("invalid execution period"));
        }
        Ok(())
    }
}

/// How a bid's accepted values relate to selected gift traits and number bounds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum BidMatch {
    /// A bid accepts at least one selected value in every selected trait group.
    #[default]
    Any,
    /// A bid has exactly the selected traits and gift-number bounds, including absent restrictions.
    All,
}

/// One range query for latest/frozen asks, bids and history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftOrderQuery {
    pub filters: GiftFilters,
    #[serde(default)]
    pub source: OrderSource,
    /// Bid-condition matching. Defaults to any; all is only valid for bids.
    #[serde(default)]
    pub bid_match: BidMatch,
    #[serde(default)]
    pub sort: Option<GiftSort>,
    pub period: Option<Period>,
    #[serde(default)]
    pub range: crate::OrderRange,
    /// Maximum rows for a finite query or a watch's initial read. Live coverage may grow to 100 rows.
    #[serde(default = "default_limit")]
    #[cfg_attr(feature = "schema", schema(minimum = 1, maximum = 100, default = 100))]
    pub limit: u32,
    #[serde(default)]
    pub view: ReadView,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum OrderSource {
    #[default]
    Asks,
    Bids,
    History,
}
impl GiftOrderQuery {
    pub fn new(filters: GiftFilters) -> Self {
        Self {
            filters,
            source: OrderSource::Asks,
            bid_match: BidMatch::Any,
            sort: None,
            period: None,
            range: crate::OrderRange::default(),
            limit: default_limit(),
            view: ReadView::Latest {},
        }
    }
    pub fn sort(&self) -> GiftSort {
        self.sort.unwrap_or(match self.source {
            OrderSource::Asks => GiftSort::PriceAsc,
            OrderSource::Bids => GiftSort::PriceDesc,
            OrderSource::History => GiftSort::TimeDesc,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ReadView {
    Latest {},
    Snapshot { id: Option<crate::ReadSnapshotId> },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ReleaseReadSnapshot {
    pub snapshot: crate::ReadSnapshotId,
}

impl Default for ReadView {
    fn default() -> Self {
        Self::Latest {}
    }
}
