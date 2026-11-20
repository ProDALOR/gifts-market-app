//! Batched current ask evidence for picker choices and order details.
use crate::{
    floors::{BookCache, Statistics},
    source::{BookScope, ValuationSnapshot},
    *,
};
use market_gifts::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[cfg(test)]
#[path = "values_tests.rs"]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct MarketValuesRequest {
    pub currency_id: CurrencyId,
    /// At most eight visible collections per watch/batch, matching the bounded shared cache.
    #[cfg_attr(feature = "schema", schema(min_items = 1, max_items = 8))]
    pub collection_ids: Vec<CollectionId>,
    /// Include all model/backdrop floors for one selected collection, plus the requested pair.
    pub traits_for: Option<CollectionId>,
    pub pair: Option<PairSelection>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PairSelection {
    pub model_id: ModelId,
    pub backdrop_id: BackdropId,
}
impl MarketValuesRequest {
    pub fn validate(&self) -> Result<(), ValuationError> {
        if self.collection_ids.is_empty()
            || self.collection_ids.len() > 8
            || self.collection_ids.iter().collect::<BTreeSet<_>>().len()
                != self.collection_ids.len()
        {
            return Err(ValuationError::invalid(
                "select 1..=8 distinct visible collections",
            ));
        }
        if self
            .traits_for
            .as_ref()
            .is_some_and(|id| !self.collection_ids.contains(id))
        {
            return Err(ValuationError::invalid(
                "traits_for must be one of the selected collections",
            ));
        }
        if self.pair.is_some() && self.traits_for.is_none() {
            return Err(ValuationError::invalid("a pair requires traits_for"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CollectionValues {
    pub collection_id: CollectionId,
    pub floor: AskFloorValuation,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PairFloor {
    pub model_id: ModelId,
    pub backdrop_id: BackdropId,
    pub floor: AskFloorValuation,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct TraitValues {
    pub collection_id: CollectionId,
    pub models: BTreeMap<ModelId, AskFloorValuation>,
    pub backdrops: BTreeMap<BackdropId, AskFloorValuation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<PairFloor>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct MarketValues {
    pub currency_id: CurrencyId,
    #[cfg_attr(feature="schema",schema(value_type=String,format=DateTime))]
    pub evaluated_at: jiff::Timestamp,
    pub stale: bool,
    pub collections: Vec<CollectionValues>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traits: Option<TraitValues>,
}
pub(crate) struct ValuesRead {
    pub values: Arc<MarketValues>,
    stats: Vec<Arc<Statistics>>,
    pub deadline: tokio::time::Instant,
}
impl ValuesRead {
    pub(crate) fn read(
        snapshot: &ValuationSnapshot,
        request: &MarketValuesRequest,
        cache: &BookCache,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, ValuationError> {
        request.validate()?;
        snapshot.require_currency(&request.currency_id)?;
        let stale = snapshot.sync()?.state != SyncState::Live;
        if let Some(previous) = previous
            && previous.values.stale == stale
            && previous
                .stats
                .iter()
                .map(|stats| stats.current(snapshot))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .all(|current| current)
        {
            return Ok(previous.clone());
        }
        let mut collections = Vec::new();
        let mut stats = Vec::new();
        let mut traits = None;
        for collection in &request.collection_ids {
            snapshot.check_cancelled()?;
            snapshot.require_collection(collection)?;
            let book = cache.get(
                snapshot,
                &BookScope {
                    collection: collection.clone(),
                    currency: request.currency_id.clone(),
                },
            )?;
            let floor = |model, backdrop| {
                let (price, evidence) = book.selection(model, backdrop);
                AskFloorValuation { price, evidence }
            };
            collections.push(CollectionValues {
                collection_id: collection.clone(),
                floor: floor(None, None),
            });
            if request.traits_for.as_ref() == Some(collection) {
                traits = Some(TraitValues {
                    collection_id: collection.clone(),
                    models: book
                        .known_models
                        .iter()
                        .map(|id| (id.clone(), floor(Some(id), None)))
                        .collect(),
                    backdrops: book
                        .known_backdrops
                        .iter()
                        .map(|id| (id.clone(), floor(None, Some(id))))
                        .collect(),
                    pair: request.pair.as_ref().map(|pair| PairFloor {
                        model_id: pair.model_id.clone(),
                        backdrop_id: pair.backdrop_id.clone(),
                        // A removed catalog choice must not stop prices for the entire batch.
                        floor: if book.known_models.contains(&pair.model_id)
                            && book.known_backdrops.contains(&pair.backdrop_id)
                        {
                            floor(Some(&pair.model_id), Some(&pair.backdrop_id))
                        } else {
                            AskFloorValuation {
                                price: market::unavailable(
                                    ValuationIssueCode::MissingTrait,
                                    "The selected pair is no longer in the collection catalog",
                                    None,
                                    None,
                                ),
                                evidence: FloorEvidence::default(),
                            }
                        },
                    }),
                });
            }
            stats.push(book);
        }
        let deadline = stats.iter().map(|stats| stats.deadline()).min().unwrap();
        // Never publish already expired evidence if another book took time to prepare.
        if deadline <= tokio::time::Instant::now() {
            return Err(ValuationError::unavailable(
                "Market asks expired during the read; retry",
            ));
        }
        Ok(Arc::new(Self {
            values: Arc::new(MarketValues {
                currency_id: request.currency_id.clone(),
                evaluated_at: jiff::Timestamp::now(),
                stale,
                collections,
                traits,
            }),
            stats,
            deadline,
        }))
    }
}
