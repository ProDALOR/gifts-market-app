use crate::{ValuationError, source::ValuationSnapshot};
use market_gifts::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationTargetKind {
    Collection,
    Model,
    Backdrop,
    ModelBackdrop,
    Gift,
    Criteria,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationTarget {
    Criteria {
        collection_id: CollectionId,
        criteria: GiftCriteria,
    },
    Collection {
        collection_id: CollectionId,
    },
    Model {
        collection_id: CollectionId,
        model_id: ModelId,
    },
    Backdrop {
        collection_id: CollectionId,
        backdrop_id: BackdropId,
    },
    ModelBackdrop {
        collection_id: CollectionId,
        model_id: ModelId,
        backdrop_id: BackdropId,
    },
    Gift {
        gift: GiftLookup,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ResolvedTarget {
    pub kind: ValuationTargetKind,
    pub collection_id: CollectionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<ModelId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backdrop_id: Option<BackdropId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gift: Option<CollectibleGift>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criteria: Option<GiftCriteria>,
}
impl ValuationTarget {
    pub(crate) fn resolve(
        &self,
        snapshot: &ValuationSnapshot,
    ) -> Result<ResolvedTarget, ValuationError> {
        if let Self::Criteria {
            collection_id,
            criteria,
        } = self
        {
            if criteria.models.len() + criteria.backdrops.len() + criteria.patterns.len() > 500 {
                return Err(ValuationError::invalid(
                    "at most 500 accepted trait alternatives",
                ));
            }
            snapshot.require_collection(collection_id)?;
            if let Some(number) = &criteria.number {
                number
                    .validate()
                    .map_err(|e| ValuationError::invalid(e.to_string()))?;
            }
            for id in &criteria.models {
                snapshot.require_trait(collection_id, crate::source::CatalogTrait::Model, id)?;
            }
            for id in &criteria.backdrops {
                snapshot.require_trait(collection_id, crate::source::CatalogTrait::Backdrop, id)?;
            }
            for id in &criteria.patterns {
                snapshot.require_trait(collection_id, crate::source::CatalogTrait::Pattern, id)?;
            }
            return Ok(ResolvedTarget {
                kind: ValuationTargetKind::Criteria,
                collection_id: collection_id.clone(),
                model_id: None,
                backdrop_id: None,
                gift: None,
                criteria: Some(criteria.clone()),
            });
        }
        let (kind, collection, model, backdrop, gift) = match self {
            Self::Criteria { .. } => unreachable!("criteria resolved above"),
            Self::Collection { collection_id } => (
                ValuationTargetKind::Collection,
                collection_id.clone(),
                None,
                None,
                None,
            ),
            Self::Model {
                collection_id,
                model_id,
            } => (
                ValuationTargetKind::Model,
                collection_id.clone(),
                Some(model_id.clone()),
                None,
                None,
            ),
            Self::Backdrop {
                collection_id,
                backdrop_id,
            } => (
                ValuationTargetKind::Backdrop,
                collection_id.clone(),
                None,
                Some(backdrop_id.clone()),
                None,
            ),
            Self::ModelBackdrop {
                collection_id,
                model_id,
                backdrop_id,
            } => (
                ValuationTargetKind::ModelBackdrop,
                collection_id.clone(),
                Some(model_id.clone()),
                Some(backdrop_id.clone()),
                None,
            ),
            Self::Gift { gift } => {
                let gift = snapshot.gift(gift)?;
                (
                    ValuationTargetKind::Gift,
                    gift.collection_id.clone(),
                    None,
                    None,
                    Some(gift),
                )
            }
        };
        snapshot.require_collection(&collection)?;
        if let Some(id) = &model {
            snapshot.require_trait(&collection, crate::source::CatalogTrait::Model, id)?;
        }
        if let Some(id) = &backdrop {
            snapshot.require_trait(&collection, crate::source::CatalogTrait::Backdrop, id)?;
        }
        Ok(ResolvedTarget {
            kind,
            collection_id: collection,
            model_id: model,
            backdrop_id: backdrop,
            gift,
            criteria: None,
        })
    }
}
impl ResolvedTarget {
    pub(crate) fn query(
        &self,
        currency: CurrencyId,
        source: OrderSource,
        period: Option<Period>,
    ) -> GiftOrderQuery {
        let mut filters = GiftFilters::new(self.collection_id.clone(), currency);
        if let Some(criteria) = &self.criteria {
            filters.models = criteria.models.clone();
            filters.backdrops = criteria.backdrops.clone();
            filters.patterns = criteria.patterns.clone();
            filters.number = criteria.number.clone();
        }
        filters.models.extend(self.model_id.clone());
        filters.backdrops.extend(self.backdrop_id.clone());
        if let Some(number) = self.gift.as_ref().and_then(|gift| gift.number) {
            filters.number = Some(GiftNumberRange {
                min: Some(number),
                max: Some(number),
            });
        }
        let mut query = GiftOrderQuery::new(filters);
        query.source = source;
        query.period = period;
        query.limit = 500;
        query
    }
}
