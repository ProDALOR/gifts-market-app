//! Catalog response projections; the hub owns catalog compilation and business rules.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalId {
    pub namespace: String,
    pub id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VenueDefinition {
    pub id: VenueId,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrencyDefinition {
    pub id: CurrencyId,
    pub name: String,
    pub decimals: u32,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<BTreeSet<AttributeValueId>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemTypeDefinition {
    pub id: ItemTypeId,
    pub name: String,
    #[serde(default = "Quantity::one")]
    pub quantity_step: Quantity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<ExternalId>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "crate::attributes::deserialize_attribute_map"
    )]
    pub attributes: BTreeMap<AttributeId, AttributeRule>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeType {
    Enum,
    Integer,
    Decimal,
    Boolean,
    Text,
    Timestamp,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeDefinition {
    pub id: AttributeId,
    pub name: String,
    pub value_type: AttributeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<AttributeValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<AttributeValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeValueDefinition {
    pub id: AttributeValueId,
    pub attribute_id: AttributeId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<ExternalId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: Metadata,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedDefinition {
    pub id: FeedId,
    pub venue_id: VenueId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CatalogEntry {
    Venue(VenueDefinition),
    Currency(CurrencyDefinition),
    ItemType(ItemTypeDefinition),
    Attribute(AttributeDefinition),
    AttributeValue(AttributeValueDefinition),
    Feed(FeedDefinition),
}
