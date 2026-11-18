//! Model floors and portfolio positions, independent of Telegram inventory retrieval.
use crate::{
    Amount, CollectionId, CurrencyId, InvalidValue, ModelId, SyncStatus, TelegramDocumentId,
    TelegramGiftId, VenueId,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ModelLookup {
    Name {
        collection: String,
        model: String,
    },
    Catalog {
        collection: CollectionId,
        model: ModelId,
    },
    Telegram {
        gift_id: TelegramGiftId,
        document_id: TelegramDocumentId,
    },
}
impl ModelLookup {
    pub fn named(
        collection: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, InvalidValue> {
        let lookup = Self::Name {
            collection: collection.into(),
            model: model.into(),
        };
        lookup.validate()?;
        Ok(lookup)
    }
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if let Self::Name { collection, model } = self
            && [collection, model]
                .iter()
                .any(|name| name.trim().is_empty() || name.len() > 256)
        {
            return Err(InvalidValue(
                "collection and model names must contain 1..256 bytes",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ModelPosition {
    pub model: ModelLookup,
    pub quantity: u32,
}
impl ModelPosition {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        if self.quantity == 0 {
            return Err(InvalidValue("portfolio quantities must be positive"));
        }
        self.model.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationRequest {
    #[serde(default)]
    pub strategy: StrategyId,
    pub positions: Vec<ModelPosition>,
    pub currency_id: CurrencyId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum UnpricedReason {
    UnknownCollection,
    AmbiguousCollection,
    UnknownModel,
    AmbiguousModel,
    NoListing,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ModelValue {
    /// Stable catalog identity; labels below remain useful for unresolved input.
    pub identity: Option<ModelIdentity>,
    pub collection: String,
    pub model: String,
    pub quantity: u32,
    pub floor: Option<Amount>,
    pub value: Option<Amount>,
    pub venue_id: Option<VenueId>,
    pub unpriced: Option<UnpricedReason>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ModelIdentity {
    pub collection_id: CollectionId,
    pub model_id: ModelId,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Valuation {
    pub rows: Vec<ModelValue>,
    pub total: Amount,
    pub priced: u32,
    pub unpriced: u32,
    pub currency_id: CurrencyId,
    pub cursor: String,
    pub sync: SyncStatus,
}
impl ValuationRequest {
    pub fn validate(&self) -> Result<(), InvalidValue> {
        let mut count = 0u32;
        for position in &self.positions {
            count = count
                .checked_add(position.quantity)
                .ok_or(InvalidValue("portfolio quantity is too large"))?;
            position.validate()?;
        }
        Ok(())
    }
}

/// Stable strategy name; unknown valid names are rejected by the running registry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature="schema",schema(value_type=String,pattern="^[a-z][a-z0-9_]{0,63}$",example="model_floor"))]
pub struct StrategyId(String);
impl StrategyId {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value.as_bytes()[0].is_ascii_lowercase()
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(InvalidValue(
                "strategy must contain 1..64 lowercase letters, digits or underscores and start with a letter",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn model_floor() -> Self {
        Self("model_floor".into())
    }
    pub fn collection_floor() -> Self {
        Self("collection_floor".into())
    }
}
impl Default for StrategyId {
    fn default() -> Self {
        Self::model_floor()
    }
}
impl std::fmt::Display for StrategyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl TryFrom<String> for StrategyId {
    type Error = InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<StrategyId> for String {
    fn from(value: StrategyId) -> Self {
        value.0
    }
}
/// Safe public context for a failed valuation. Rows explain individual missing quotes/identities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationErrorDetails {
    pub strategy: StrategyId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency_id: Option<CurrencyId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_samples: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_samples: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpriced: Vec<ModelValue>,
}
impl ValuationErrorDetails {
    pub fn strategy(strategy: StrategyId) -> Self {
        Self {
            strategy,
            currency_id: None,
            revision: None,
            required_samples: None,
            available_samples: None,
            unpriced: vec![],
        }
    }
}
