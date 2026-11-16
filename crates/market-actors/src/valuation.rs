//! Floor estimates shared by bot and API callers. No Telegram transport or ownership storage.
use crate::{
    SyncStatus,
    error::{DataError, Result},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelLookup {
    /// None matches catalog IDs or exact names; Some matches catalog aliases in this namespace.
    pub namespace: Option<String>,
    pub collection: String,
    pub attribute_id: String,
    pub model: String,
}
#[derive(Debug, Clone)]
pub struct ModelPosition {
    pub model: ModelLookup,
    pub quantity: u32,
}
pub struct ValuationRequest {
    pub positions: Vec<ModelPosition>,
    pub currency_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnpricedReason {
    UnknownCollection,
    AmbiguousCollection,
    UnknownModel,
    AmbiguousModel,
    NoListing,
}
#[derive(Debug, Clone)]
pub struct ModelValue {
    pub collection: String,
    pub model: String,
    pub quantity: u32,
    pub floor: Option<String>,
    pub value: Option<String>,
    pub venue_id: Option<String>,
    pub unpriced: Option<UnpricedReason>,
}
pub struct Valuation {
    pub rows: Vec<ModelValue>,
    pub total: String,
    pub priced: u32,
    pub unpriced: u32,
    pub currency_id: String,
    pub cursor: String,
    pub sync: SyncStatus,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ModelKey {
    pub collection: String,
    pub attribute: String,
    pub option: String,
}
pub(crate) struct ResolvedPosition {
    pub key: Option<ModelKey>,
    pub row: ModelValue,
}
pub(crate) fn validate(request: &ValuationRequest) -> Result<()> {
    if request.positions.len() > 1000
        || request.currency_id.is_empty()
        || request.currency_id.len() > 128
    {
        return Err(DataError::invalid(
            "at most 1000 model positions and a quote currency are required",
        ));
    }
    let mut count = 0u32;
    for position in &request.positions {
        count = count
            .checked_add(position.quantity)
            .ok_or_else(|| DataError::invalid("portfolio quantity is too large"))?;
        if position.quantity == 0
            || position
                .model
                .namespace
                .as_ref()
                .is_some_and(|namespace| namespace.is_empty() || namespace.len() > 64)
            || count > 10_000
            || [
                &position.model.collection,
                &position.model.model,
                &position.model.attribute_id,
            ]
            .iter()
            .any(|s| s.is_empty() || s.len() > 256)
        {
            return Err(DataError::invalid("invalid model position"));
        }
    }
    Ok(())
}
