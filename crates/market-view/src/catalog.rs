//! Read-side lookup tables and query validation, built from a complete gRPC catalog revision.
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CatalogDefinition {
    pub venues: Vec<VenueDefinition>,
    pub currencies: Vec<CurrencyDefinition>,
    pub item_types: Vec<ItemTypeDefinition>,
    pub attributes: Vec<AttributeDefinition>,
    pub attribute_values: Vec<AttributeValueDefinition>,
    pub feeds: Vec<FeedDefinition>,
}
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    venues: BTreeMap<VenueId, VenueDefinition>,
    currencies: BTreeMap<CurrencyId, CurrencyDefinition>,
    types: BTreeMap<ItemTypeId, ItemTypeDefinition>,
    attributes: BTreeMap<AttributeId, AttributeDefinition>,
    values: BTreeMap<AttributeValueId, AttributeValueDefinition>,
    feeds: BTreeMap<FeedId, FeedDefinition>,
}
impl Serialize for Catalog {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.definition().serialize(s)
    }
}
impl CatalogEntry {
    pub fn identity(&self) -> (u8, String) {
        match self {
            Self::Venue(v) => (1, v.id.to_string()),
            Self::Currency(v) => (2, v.id.to_string()),
            Self::ItemType(v) => (3, v.id.to_string()),
            Self::Attribute(v) => (4, v.id.to_string()),
            Self::AttributeValue(v) => (5, v.id.to_string()),
            Self::Feed(v) => (6, v.id.to_string()),
        }
    }
}
impl Catalog {
    pub fn compile(d: CatalogDefinition) -> Result<Self, ValidationError> {
        Self::from_entries(
            d.venues
                .into_iter()
                .map(CatalogEntry::Venue)
                .chain(d.currencies.into_iter().map(CatalogEntry::Currency))
                .chain(d.item_types.into_iter().map(CatalogEntry::ItemType))
                .chain(d.attributes.into_iter().map(CatalogEntry::Attribute))
                .chain(
                    d.attribute_values
                        .into_iter()
                        .map(CatalogEntry::AttributeValue),
                )
                .chain(d.feeds.into_iter().map(CatalogEntry::Feed)),
        )
    }
    pub fn from_entries(
        entries: impl IntoIterator<Item = CatalogEntry>,
    ) -> Result<Self, ValidationError> {
        let mut s = Self::default();
        for e in entries {
            let duplicate = match e {
                CatalogEntry::Venue(v) => s.venues.insert(v.id.clone(), v).is_some(),
                CatalogEntry::Currency(v) => s.currencies.insert(v.id.clone(), v).is_some(),
                CatalogEntry::ItemType(v) => s.types.insert(v.id.clone(), v).is_some(),
                CatalogEntry::Attribute(v) => s.attributes.insert(v.id.clone(), v).is_some(),
                CatalogEntry::AttributeValue(v) => s.values.insert(v.id.clone(), v).is_some(),
                CatalogEntry::Feed(v) => s.feeds.insert(v.id.clone(), v).is_some(),
            };
            if duplicate {
                return Err(ValidationError("duplicate catalog identity"));
            }
        }
        for f in s.feeds.values() {
            if !s.venues.contains_key(&f.venue_id) {
                return Err(ValidationError("feed has unknown venue"));
            }
        }
        for v in s.values.values() {
            if s.attribute(&v.attribute_id)
                .is_none_or(|a| a.value_type != AttributeType::Enum)
            {
                return Err(ValidationError("option has unknown enum attribute"));
            }
        }
        for t in s.types.values() {
            for (id, r) in &t.attributes {
                if s.attribute(id).is_none() {
                    return Err(ValidationError("unknown bound attribute"));
                }
                if let Some(options) = &r.allowed_values
                    && (options.is_empty()
                        || options
                            .iter()
                            .any(|v| s.attribute_value(v).is_none_or(|v| &v.attribute_id != id)))
                {
                    return Err(ValidationError("invalid allowed attribute values"));
                }
            }
        }
        Ok(s)
    }
    pub fn definition(&self) -> CatalogDefinition {
        CatalogDefinition {
            venues: self.venues.values().cloned().collect(),
            currencies: self.currencies.values().cloned().collect(),
            item_types: self.types.values().cloned().collect(),
            attributes: self.attributes.values().cloned().collect(),
            attribute_values: self.values.values().cloned().collect(),
            feeds: self.feeds.values().cloned().collect(),
        }
    }
    pub fn entries(&self) -> impl Iterator<Item = CatalogEntry> + '_ {
        self.venues
            .values()
            .cloned()
            .map(CatalogEntry::Venue)
            .chain(
                self.currencies
                    .values()
                    .cloned()
                    .map(CatalogEntry::Currency),
            )
            .chain(self.types.values().cloned().map(CatalogEntry::ItemType))
            .chain(
                self.attributes
                    .values()
                    .cloned()
                    .map(CatalogEntry::Attribute),
            )
            .chain(
                self.values
                    .values()
                    .cloned()
                    .map(CatalogEntry::AttributeValue),
            )
            .chain(self.feeds.values().cloned().map(CatalogEntry::Feed))
    }
    pub fn feed(&self, id: &FeedId) -> Option<&FeedDefinition> {
        self.feeds.get(id)
    }
    pub fn feeds(&self) -> impl Iterator<Item = &FeedDefinition> {
        self.feeds.values()
    }
    pub fn item_type(&self, id: &ItemTypeId) -> Option<&ItemTypeDefinition> {
        self.types.get(id)
    }
    pub fn attribute(&self, id: &AttributeId) -> Option<&AttributeDefinition> {
        self.attributes.get(id)
    }
    pub fn attribute_value(&self, id: &AttributeValueId) -> Option<&AttributeValueDefinition> {
        self.values.get(id)
    }
    pub fn currency(&self, id: &CurrencyId) -> Option<&CurrencyDefinition> {
        self.currencies.get(id)
    }
    pub fn validate_money(&self, v: &Money) -> Result<(), ValidationError> {
        v.validate()?;
        let c = self
            .currency(&v.currency_id)
            .ok_or(ValidationError("unknown currency"))?;
        if v.amount.normalize().scale() > c.decimals {
            return Err(ValidationError("price exceeds currency precision"));
        }
        Ok(())
    }
    fn check_value(
        &self,
        kind: &ItemTypeId,
        id: &AttributeId,
        v: &AttributeValue,
    ) -> Result<(), ValidationError> {
        v.validate()?;
        let t = self
            .item_type(kind)
            .ok_or(ValidationError("unknown item type ID"))?;
        let rule = t.attributes.get(id).ok_or(ValidationError(
            "attribute is not permitted for this item type",
        ))?;
        let a = self
            .attribute(id)
            .ok_or(ValidationError("unknown attribute ID"))?;
        if v.value_type() != a.value_type {
            return Err(ValidationError("attribute type mismatch"));
        }
        if let AttributeValue::Enum(value) = v
            && (self
                .attribute_value(value)
                .is_none_or(|e| &e.attribute_id != id)
                || rule
                    .allowed_values
                    .as_ref()
                    .is_some_and(|allowed| !allowed.contains(value)))
        {
            return Err(ValidationError("attribute option not allowed"));
        }
        if a.minimum
            .as_ref()
            .is_some_and(|min| v.range_cmp(min).is_none_or(|c| c.is_lt()))
            || a.maximum
                .as_ref()
                .is_some_and(|max| v.range_cmp(max).is_none_or(|c| c.is_gt()))
        {
            return Err(ValidationError("attribute outside bounds"));
        }
        if let AttributeValue::Text(s) = v
            && a.max_length.is_some_and(|n| s.len() > n)
        {
            return Err(ValidationError("text exceeds schema limit"));
        }
        Ok(())
    }
    pub fn validate_attributes(
        &self,
        kind: &ItemTypeId,
        a: &Attributes,
    ) -> Result<(), ValidationError> {
        validate_attributes(a)?;
        if self.item_type(kind).is_none() {
            return Err(ValidationError("unknown item type ID"));
        }
        for (id, v) in a {
            self.check_value(kind, id, v)?
        }
        Ok(())
    }
    pub fn validate_filter(
        &self,
        kind: &ItemTypeId,
        a: &AttributeFilter,
    ) -> Result<(), ValidationError> {
        validate_attribute_filter(a)?;
        for (id, p) in a {
            for v in p.values() {
                self.check_value(kind, id, v)?
            }
        }
        Ok(())
    }
    pub fn validate_attribute_filter(
        &self,
        kind: &ItemTypeId,
        a: &AttributeFilter,
    ) -> Result<(), ValidationError> {
        self.validate_filter(kind, a)
    }
    pub fn validate_amounts(
        &self,
        kind: &ItemTypeId,
        a: &PriceQuantity,
    ) -> Result<(), ValidationError> {
        self.validate_money(&a.price)?;
        if !a.quantity.is_multiple_of(
            self.item_type(kind)
                .ok_or(ValidationError("unknown item type ID"))?
                .quantity_step,
        ) {
            return Err(ValidationError("quantity does not match step"));
        }
        Ok(())
    }
    pub fn validate_mutation(&self, m: &Mutation) -> Result<(), ValidationError> {
        let feed = self
            .feed(&m.key().feed_id)
            .ok_or(ValidationError("unknown feed ID"))?;
        match m {
            Mutation::Delete(k) => k.validate(),
            Mutation::Upsert(Record::Feed(v)) => {
                if v.venue_id == feed.venue_id {
                    Ok(())
                } else {
                    Err(ValidationError("feed venue differs"))
                }
            }
            Mutation::Upsert(Record::Item(v)) => {
                self.validate_attributes(&v.item_type_id, &v.attributes)
            }
            Mutation::Upsert(Record::Order(v)) => {
                v.validate()?;
                self.validate_amounts(&v.item_type_id, &v.amounts)?;
                match &v.kind {
                    OrderKind::AskUnits { attributes } => {
                        self.validate_attributes(&v.item_type_id, attributes)
                    }
                    OrderKind::BidAttributes { filter } => {
                        self.validate_filter(&v.item_type_id, filter)
                    }
                    _ => Ok(()),
                }
            }
        }
    }
    pub fn validate_source(&self, v: &SourceEvent) -> Result<(), ValidationError> {
        v.validate()
    }
}
impl Catalog {
    pub fn validate_event(&self, event: &MarketEvent) -> Result<(), ValidationError> {
        event.validate()?;
        match event {
            MarketEvent::Upsert(v) => self.validate_mutation(&Mutation::Upsert(v.clone())),
            MarketEvent::Delete(v) => self.validate_mutation(&Mutation::Delete(v.clone())),
            MarketEvent::Trade(v) => {
                self.validate_amounts(&v.item_type_id, &v.amounts)?;
                if let TradeKind::Units { attributes } = &v.kind {
                    self.validate_attributes(&v.item_type_id, attributes)?
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
impl CatalogDefinition {
    pub fn into_entries(self) -> impl Iterator<Item = CatalogEntry> {
        self.venues
            .into_iter()
            .map(CatalogEntry::Venue)
            .chain(self.currencies.into_iter().map(CatalogEntry::Currency))
            .chain(self.item_types.into_iter().map(CatalogEntry::ItemType))
            .chain(self.attributes.into_iter().map(CatalogEntry::Attribute))
            .chain(
                self.attribute_values
                    .into_iter()
                    .map(CatalogEntry::AttributeValue),
            )
            .chain(self.feeds.into_iter().map(CatalogEntry::Feed))
    }
}
