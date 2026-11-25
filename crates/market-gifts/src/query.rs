//! Gift query contracts and reusable range and period invariants.
use crate::{
    Amount, BackdropId, BackdropRef, CollectionId, CollectionRef, CurrencyId, GiftNumber,
    InvalidValue, ModelId, ModelRef, PatternId, PatternRef, VenueId,
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
#[serde(bound(
    deserialize = "C: Deserialize<'de>, M: Deserialize<'de>, B: Deserialize<'de>, P: Deserialize<'de>"
))]
pub struct GiftFilters<C = CollectionRef, M = ModelRef, B = BackdropRef, P = PatternRef> {
    /// Native collection ID or @namespace:value selector; omit to query all gift collections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_id: Option<C>,
    pub currency_id: CurrencyId,
    pub venue_id: Option<VenueId>,
    #[serde(default)]
    /// Native IDs or aliases, resolved only inside the selected collection.
    pub models: Vec<M>,
    #[serde(default)]
    /// Native IDs or shared aliases; alternatives match within every selected collection.
    pub backdrops: Vec<B>,
    #[serde(default)]
    /// Symbol option IDs or shared aliases, independent of backdrop identities.
    pub patterns: Vec<P>,
    pub number: Option<GiftNumberRange>,
    pub price: Option<PriceRange>,
}
pub type ResolvedGiftFilters = GiftFilters<CollectionId, ModelId, BackdropId, PatternId>;
impl<C, M, B, P> GiftFilters<C, M, B, P> {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if let Some(range) = &self.number {
            range.validate()?;
        }
        if let Some(range) = &self.price {
            range.validate()?;
        }
        Ok(())
    }
    pub fn new(collection_id: C, currency_id: CurrencyId) -> Self {
        Self {
            collection_id: Some(collection_id),
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
pub struct GiftOrderQuery<F = GiftFilters> {
    pub filters: F,
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
pub type ResolvedGiftOrderQuery = GiftOrderQuery<ResolvedGiftFilters>;
impl<F> GiftOrderQuery<F> {
    pub fn new(filters: F) -> Self {
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
    pub fn validate_context(&self) -> Result<(), InvalidValue> {
        if self.source != OrderSource::Bids && self.bid_match != BidMatch::Any {
            return Err(InvalidValue("bid_match all is only valid for bids"));
        }
        if self.source != OrderSource::History && self.period.is_some() {
            return Err(InvalidValue("period is only valid for history"));
        }
        if let Some(period) = &self.period {
            period.validate()?;
        }
        Ok(())
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

impl From<ResolvedGiftFilters> for GiftFilters {
    fn from(filters: ResolvedGiftFilters) -> Self {
        Self {
            collection_id: filters.collection_id.map(Into::into),
            currency_id: filters.currency_id,
            venue_id: filters.venue_id,
            models: filters.models.into_iter().map(Into::into).collect(),
            backdrops: filters.backdrops.into_iter().map(Into::into).collect(),
            patterns: filters.patterns.into_iter().map(Into::into).collect(),
            number: filters.number,
            price: filters.price,
        }
    }
}
impl<C, M, B, P> GiftFilters<C, M, B, P>
where
    C: crate::CatalogReference<Id = CollectionId>,
    M: crate::CatalogReference<Id = ModelId>,
    B: crate::CatalogReference<Id = BackdropId>,
    P: crate::CatalogReference<Id = PatternId>,
{
    pub fn has_aliases(&self) -> bool {
        self.collection_id
            .as_ref()
            .is_some_and(crate::CatalogReference::is_alias)
            || self.models.iter().any(crate::CatalogReference::is_alias)
            || self.backdrops.iter().any(crate::CatalogReference::is_alias)
            || self.patterns.iter().any(crate::CatalogReference::is_alias)
    }
    pub fn into_resolved(self) -> Result<ResolvedGiftFilters, crate::AliasError> {
        Ok(ResolvedGiftFilters {
            collection_id: self
                .collection_id
                .map(crate::CatalogReference::into_id)
                .transpose()?,
            currency_id: self.currency_id,
            venue_id: self.venue_id,
            models: self
                .models
                .into_iter()
                .map(crate::CatalogReference::into_id)
                .collect::<Result<_, _>>()?,
            backdrops: self
                .backdrops
                .into_iter()
                .map(crate::CatalogReference::into_id)
                .collect::<Result<_, _>>()?,
            patterns: self
                .patterns
                .into_iter()
                .map(crate::CatalogReference::into_id)
                .collect::<Result<_, _>>()?,
            number: self.number,
            price: self.price,
        })
    }
}
impl<F> GiftOrderQuery<F> {
    pub fn map_filters<T>(self, filters: T) -> GiftOrderQuery<T> {
        GiftOrderQuery {
            filters,
            source: self.source,
            bid_match: self.bid_match,
            sort: self.sort,
            period: self.period,
            range: self.range,
            limit: self.limit,
            view: self.view,
        }
    }
}

impl From<ResolvedGiftOrderQuery> for GiftOrderQuery {
    fn from(query: ResolvedGiftOrderQuery) -> Self {
        let filters = query.filters.clone().into();
        query.map_filters(filters)
    }
}
