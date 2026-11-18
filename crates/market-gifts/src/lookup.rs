//! Shared catalog identities and lookup inputs, independent of valuation policies.
use crate::{CollectionId, InvalidValue, ModelId, TelegramDocumentId, TelegramGiftId};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ModelLookup {
    /// Collection-only floor lookup; no model identity is invented for a partial gift.
    Collection {
        collection: CollectionId,
    },
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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ModelIdentity {
    pub collection_id: CollectionId,
    pub model_id: ModelId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ModelLookupError {
    UnknownCollection,
    AmbiguousCollection,
    UnknownModel,
    AmbiguousModel,
}
