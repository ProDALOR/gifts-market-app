//! Batched current ask floors, retaining per-collection calculation freshness.
use crate::*;
use market_gifts::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
#[cfg(test)]
#[path = "floor_projection_tests.rs"]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CollectionFloors {
    pub collection_id: CollectionId,
    /// When this collection's current statistics were calculated; unchanged by other collections.
    #[cfg_attr(feature="schema",schema(value_type=String,format=DateTime))]
    pub evaluated_at: jiff::Timestamp,
    pub floor: FloorValuation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traits: Option<TraitFloors>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PairFloor {
    pub model_id: ModelId,
    pub backdrop_id: BackdropId,
    pub floor: FloorValuation,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct TraitFloors {
    pub models: BTreeMap<ModelId, FloorValuation>,
    pub backdrops: BTreeMap<BackdropId, FloorValuation>,
    /// Current ask floors for symbols, independent of the gift-floor estimate formula.
    pub patterns: BTreeMap<PatternId, FloorValuation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<PairFloor>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CatalogFloorValuation {
    pub collections: Vec<CollectionFloors>,
}
/// The picker projection is an ordinary strategy with catalog-scoped typed input.
pub struct CatalogFloor;
impl Strategy for CatalogFloor {
    const PURPOSE: crate::StrategyPurpose = crate::StrategyPurpose::Reference;
    const DISPLAY: crate::DisplayDefinition = crate::DisplayDefinition {
        fields: &[],
        evidence: &[],
        note: "Current eligible asks, before fees.",
    };
    type Input = CatalogInput;
    type Output = Arc<CatalogFloorValuation>;
    const ID: &'static str = "catalog_floor";
    const NAME: &'static str = "Catalog best asks";
    const DESCRIPTION: &'static str = "Batched collection, model, backdrop and symbol ask floors.";
    const DOCUMENTATION: &'static str = include_str!("catalog_floor.md");
    fn evaluate(
        &self,
        input: &Self::Input,
        context: &ValuationContext<'_>,
    ) -> Result<Self::Output, ValuationError> {
        let request = input.selection();
        let mut collections = Vec::with_capacity(request.collection_ids.len());
        for collection in &request.collection_ids {
            context.snapshot().check_cancelled()?;
            let details = request
                .details
                .as_ref()
                .filter(|details| &details.collection_id == collection);
            let book = if details.is_some() {
                context.floor_statistics(collection)?
            } else {
                context.collection_statistics(collection)?
            };
            let traits = if let Some(details) = details {
                let pair = if let Some(pair) = &details.pair {
                    Some(PairFloor {
                        model_id: pair.model_id.clone(),
                        backdrop_id: pair.backdrop_id.clone(),
                        floor: if book.known_models.contains(&pair.model_id)
                            && book.known_backdrops.contains(&pair.backdrop_id)
                        {
                            book.pair_floor(context.snapshot(), &pair.model_id, &pair.backdrop_id)?
                        } else {
                            FloorValuation {
                                price: market::unavailable(
                                    ValuationIssueCode::MissingTrait,
                                    "The selected pair is no longer in the collection catalog",
                                    None,
                                    None,
                                ),
                                count: 0,
                            }
                        },
                    })
                } else {
                    None
                };
                Some(TraitFloors {
                    models: book
                        .known_models
                        .iter()
                        .map(|id| {
                            (
                                id.clone(),
                                book.selection(crate::floors::FloorSelection::Model(id)),
                            )
                        })
                        .collect(),
                    backdrops: book
                        .known_backdrops
                        .iter()
                        .map(|id| {
                            (
                                id.clone(),
                                book.selection(crate::floors::FloorSelection::Backdrop(id)),
                            )
                        })
                        .collect(),
                    patterns: book
                        .known_patterns
                        .iter()
                        .map(|id| {
                            (
                                id.clone(),
                                book.selection(crate::floors::FloorSelection::Pattern(id)),
                            )
                        })
                        .collect(),
                    pair,
                })
            } else {
                None
            };
            collections.push(CollectionFloors {
                collection_id: collection.clone(),
                evaluated_at: book.evaluated_at,
                floor: book.selection(crate::floors::FloorSelection::Collection),
                traits,
            });
        }
        Ok(Arc::new(CatalogFloorValuation { collections }))
    }
}
