//! Batched current ask floors, retaining per-collection calculation freshness.
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
#[path = "floor_projection_tests.rs"]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct FloorQuery {
    pub currency_id: CurrencyId,
    /// One to eight distinct collections in this currency.
    #[cfg_attr(feature = "schema", schema(min_items = 1, max_items = 8))]
    pub collection_ids: Vec<CollectionId>,
    /// Include all known models/backdrops and an optional pair for one collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<FloorDetails>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct FloorDetails {
    pub collection_id: CollectionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<PairSelection>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PairSelection {
    pub model_id: ModelId,
    pub backdrop_id: BackdropId,
}
impl FloorQuery {
    pub fn validate(&self) -> Result<(), ValuationError> {
        if self.collection_ids.is_empty()
            || self.collection_ids.len() > 8
            || self.collection_ids.iter().collect::<BTreeSet<_>>().len()
                != self.collection_ids.len()
        {
            return Err(ValuationError::invalid("select 1..=8 distinct collections"));
        }
        if self
            .details
            .as_ref()
            .is_some_and(|details| !self.collection_ids.contains(&details.collection_id))
        {
            return Err(ValuationError::invalid(
                "details.collection_id must be in collection_ids",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CollectionFloors {
    pub collection_id: CollectionId,
    /// When this collection's current statistics were calculated; unchanged by other collections.
    #[cfg_attr(feature="schema",schema(value_type=String,format=DateTime))]
    pub evaluated_at: jiff::Timestamp,
    pub floor: AskFloorValuation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traits: Option<TraitFloors>,
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
pub struct TraitFloors {
    pub models: BTreeMap<ModelId, AskFloorValuation>,
    pub backdrops: BTreeMap<BackdropId, AskFloorValuation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<PairFloor>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct FloorReport {
    pub currency_id: CurrencyId,
    pub stale: bool,
    pub collections: Vec<CollectionFloors>,
}
pub(crate) struct FloorRead {
    pub values: Arc<FloorReport>,
    stats: Vec<Arc<Statistics>>,
    pub deadline: tokio::time::Instant,
}
impl FloorRead {
    pub(crate) fn read(
        snapshot: &ValuationSnapshot,
        request: &FloorQuery,
        cache: &BookCache,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, ValuationError> {
        request.validate()?;
        snapshot.require_currency(&request.currency_id)?;
        let stale = snapshot.sync()?.state != SyncState::Live;
        if let Some(previous) = previous {
            let mut current = previous.values.stale == stale;
            for stats in &previous.stats {
                if !current {
                    break;
                }
                current = stats.current(snapshot)?;
            }
            if current {
                return Ok(previous.clone());
            }
        }
        let mut collections = Vec::with_capacity(request.collection_ids.len());
        let mut stats = Vec::with_capacity(request.collection_ids.len());
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
            let traits = request
                .details
                .as_ref()
                .filter(|details| &details.collection_id == collection)
                .map(|details| TraitFloors {
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
                    pair: details.pair.as_ref().map(|pair| PairFloor {
                        model_id: pair.model_id.clone(),
                        backdrop_id: pair.backdrop_id.clone(),
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
            collections.push(CollectionFloors {
                collection_id: collection.clone(),
                evaluated_at: book.evaluated_at,
                floor: floor(None, None),
                traits,
            });
            stats.push(book);
        }
        let deadline = stats.iter().map(|stats| stats.deadline()).min().unwrap();
        if deadline <= tokio::time::Instant::now() {
            return Err(ValuationError::unavailable(
                "Asks expired during the read; retry",
            ));
        }
        Ok(Arc::new(Self {
            values: Arc::new(FloorReport {
                currency_id: request.currency_id.clone(),
                stale,
                collections,
            }),
            stats,
            deadline,
        }))
    }
}
