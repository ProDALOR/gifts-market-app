//! Gift query contracts and reusable range and period invariants.
use crate::{
    Amount, BackdropId, CollectionId, CurrencyId, GiftSide, InvalidValue, ModelId, PatternId,
    ReadConsistency, VenueId,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

fn default_limit() -> u32 {
    50
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
    #[cfg_attr(feature = "schema", schema(minimum = 1))]
    pub min: Option<u32>,
    #[cfg_attr(feature = "schema", schema(minimum = 1))]
    pub max: Option<u32>,
}
impl GiftNumberRange {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if self.min == Some(0)
            || self.max == Some(0)
            || self.min.zip(self.max).is_some_and(|(min, max)| min > max)
        {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftOrdersQuery {
    pub filters: GiftFilters,
    #[serde(default)]
    pub side: GiftSide,
    #[serde(default)]
    pub sort: GiftSort,
    #[serde(default)]
    pub consistency: ReadConsistency,
    #[serde(default = "default_limit")]
    #[cfg_attr(feature = "schema", schema(minimum = 1, maximum = 200))]
    pub limit: u32,
    #[serde(default)]
    pub page_token: String,
}
impl GiftOrdersQuery {
    pub fn new(filters: GiftFilters) -> Self {
        Self {
            filters,
            side: Default::default(),
            sort: Default::default(),
            consistency: Default::default(),
            limit: default_limit(),
            page_token: String::new(),
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftHistoryQuery {
    pub filters: GiftFilters,
    #[serde(default = "history_sort")]
    pub sort: GiftSort,
    pub period: Option<Period>,
    #[serde(default = "default_limit")]
    #[cfg_attr(feature = "schema", schema(minimum = 1, maximum = 200))]
    pub limit: u32,
    #[serde(default)]
    pub page_token: String,
}
fn history_sort() -> GiftSort {
    GiftSort::Identity
}
impl GiftHistoryQuery {
    pub fn new(filters: GiftFilters) -> Self {
        Self {
            filters,
            sort: GiftSort::Identity,
            period: None,
            limit: default_limit(),
            page_token: String::new(),
        }
    }
}
