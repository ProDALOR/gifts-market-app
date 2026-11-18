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
