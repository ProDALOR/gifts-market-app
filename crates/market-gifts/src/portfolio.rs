//! Portfolio subjects and selections, independent of inventory fetching and actors.
use crate::{
    BackdropId, BackdropRef, CollectionId, CollectionRef, CurrencyId, GiftNumber, InvalidValue,
    ModelId, ModelRef, PatternId, PatternRef,
};
use serde::{Deserialize, Serialize};
#[cfg(feature = "schema")]
use utoipa::ToSchema;
const MAX_USER_ID: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct UserId(u64);
#[cfg(feature = "schema")]
impl utoipa::PartialSchema for UserId {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Integer)
            .minimum(Some(1u64))
            .maximum(Some(MAX_USER_ID))
            .into()
    }
}
#[cfg(feature = "schema")]
impl ToSchema for UserId {}
impl UserId {
    pub fn new(value: u64) -> Result<Self, InvalidValue> {
        if !(1..=MAX_USER_ID).contains(&value) {
            return Err(InvalidValue("invalid Telegram user ID"));
        }
        Ok(Self(value))
    }
    pub fn get(self) -> u64 {
        self.0
    }
}
impl TryFrom<u64> for UserId {
    type Error = InvalidValue;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<UserId> for u64 {
    fn from(value: UserId) -> u64 {
        value.0
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(bound(
    deserialize = "C: Deserialize<'de>, M: Deserialize<'de>, B: Deserialize<'de>, P: Deserialize<'de>"
))]
pub struct PortfolioFilters<C = CollectionRef, M = ModelRef, B = BackdropRef, P = PatternRef> {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schema(required = false))]
    pub collection_id: Option<C>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schema(max_items = 4096))]
    pub models: Vec<M>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schema(max_items = 4096))]
    pub backdrops: Vec<B>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schema(max_items = 4096))]
    pub patterns: Vec<P>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schema(required = false))]
    pub number: Option<GiftNumber>,
}
pub type ResolvedPortfolioFilters = PortfolioFilters<CollectionId, ModelId, BackdropId, PatternId>;
impl<C, M, B, P> Default for PortfolioFilters<C, M, B, P> {
    fn default() -> Self {
        Self {
            collection_id: None,
            models: vec![],
            backdrops: vec![],
            patterns: vec![],
            number: None,
        }
    }
}
impl<C, M: Ord, B: Ord, P: Ord> PortfolioFilters<C, M, B, P> {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if self.collection_id.is_none() && !self.models.is_empty() {
            return Err(InvalidValue("model filters require a collection"));
        }
        if [self.models.len(), self.backdrops.len(), self.patterns.len()]
            .into_iter()
            .any(|n| n > 4096)
        {
            return Err(InvalidValue("select at most 4096 values per trait"));
        }
        Ok(())
    }
    pub fn normalize(&mut self) -> Result<(), InvalidValue> {
        self.validate()?;
        self.models.sort();
        self.models.dedup();
        self.backdrops.sort();
        self.backdrops.dedup();
        self.patterns.sort();
        self.patterns.dedup();
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum PortfolioSort {
    PositionAsc,
    PositionDesc,
    NumberAsc,
    NumberDesc,
    ValueAsc,
    #[default]
    ValueDesc,
}
/// A reusable portfolio selection, independent of page coverage and inventory versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(bound(
    deserialize = "C: Deserialize<'de>, M: Deserialize<'de>, B: Deserialize<'de>, P: Deserialize<'de>"
))]
pub struct PortfolioTarget<C = CollectionRef, M = ModelRef, B = BackdropRef, P = PatternRef> {
    pub user_id: UserId,
    pub currency_id: CurrencyId,
    #[serde(default)]
    pub filters: PortfolioFilters<C, M, B, P>,
    #[serde(default)]
    pub sort: PortfolioSort,
}
pub type ResolvedPortfolioTarget = PortfolioTarget<CollectionId, ModelId, BackdropId, PatternId>;
impl<C, M: Ord, B: Ord, P: Ord> PortfolioTarget<C, M, B, P> {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        self.filters.validate()
    }
}
