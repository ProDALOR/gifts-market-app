use crate::{
    ValuationError,
    source::{CatalogTrait, ValuationSnapshot},
};
use market_gifts::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationTargetKind {
    Catalog,
    Collection,
    Model,
    Backdrop,
    ModelBackdrop,
    Gift,
    Criteria,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationTarget {
    Catalog(crate::CatalogSelection),
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
    Criteria {
        collection_id: CollectionId,
        criteria: GiftCriteria,
    },
    Gift {
        gift: GiftLookup,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CatalogSelection {
    /// One to eight distinct collections in this currency.
    #[cfg_attr(feature = "schema", schema(min_items = 1, max_items = 8))]
    pub collection_ids: Vec<CollectionId>,
    /// Include all known models/backdrops and an optional pair for one collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<CatalogDetails>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CatalogDetails {
    pub collection_id: CollectionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<PairSelection>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PairSelection {
    pub model_id: ModelId,
    pub backdrop_id: BackdropId,
}
impl CatalogSelection {
    pub fn validate(&self) -> Result<(), ValuationError> {
        if self.collection_ids.is_empty()
            || self.collection_ids.len() > 8
            || self
                .collection_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
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

/// A resolved target has exactly the fields required by its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ResolvedTarget {
    Catalog(crate::CatalogSelection),
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
    Criteria {
        collection_id: CollectionId,
        criteria: GiftCriteria,
    },
    Gift {
        gift: CollectibleGift,
        #[serde(skip_serializing_if = "Option::is_none")]
        slug: Option<GiftSlug>,
    },
}
impl ValuationTarget {
    pub(crate) fn resolve(
        &self,
        snapshot: &ValuationSnapshot,
    ) -> Result<ResolvedTarget, ValuationError> {
        if let Self::Catalog(selection) = self {
            for collection in &selection.collection_ids {
                snapshot.require_collection(collection)?;
            }
            return Ok(ResolvedTarget::Catalog(selection.clone()));
        }
        if let Self::Gift { gift: lookup } = self {
            return Ok(ResolvedTarget::Gift {
                gift: snapshot.gift(lookup)?,
                slug: match lookup {
                    GiftLookup::Slug(slug) => Some(slug.clone()),
                    _ => None,
                },
            });
        }
        let resolved = match self {
            Self::Collection { collection_id } => ResolvedTarget::Collection {
                collection_id: collection_id.clone(),
            },
            Self::Model {
                collection_id,
                model_id,
            } => ResolvedTarget::Model {
                collection_id: collection_id.clone(),
                model_id: model_id.clone(),
            },
            Self::Backdrop {
                collection_id,
                backdrop_id,
            } => ResolvedTarget::Backdrop {
                collection_id: collection_id.clone(),
                backdrop_id: backdrop_id.clone(),
            },
            Self::ModelBackdrop {
                collection_id,
                model_id,
                backdrop_id,
            } => ResolvedTarget::ModelBackdrop {
                collection_id: collection_id.clone(),
                model_id: model_id.clone(),
                backdrop_id: backdrop_id.clone(),
            },
            Self::Criteria {
                collection_id,
                criteria,
            } => ResolvedTarget::Criteria {
                collection_id: collection_id.clone(),
                criteria: criteria.clone(),
            },
            Self::Gift { .. } | Self::Catalog(_) => unreachable!(),
        };
        let collection = resolved.collection().expect("single target");
        snapshot.require_collection(collection)?;
        if let Some(id) = resolved.model() {
            snapshot.require_trait(collection, CatalogTrait::Model, id)?;
        }
        if let Some(id) = resolved.backdrop() {
            snapshot.require_trait(collection, CatalogTrait::Backdrop, id)?;
        }
        if let Some(criteria) = resolved.criteria() {
            if criteria.models.len() + criteria.backdrops.len() + criteria.patterns.len() > 500 {
                return Err(ValuationError::invalid(
                    "at most 500 accepted trait alternatives",
                ));
            }
            if let Some(number) = &criteria.number {
                number
                    .validate()
                    .map_err(|e| ValuationError::invalid(e.to_string()))?;
            }
            for id in &criteria.models {
                snapshot.require_trait(collection, CatalogTrait::Model, id)?;
            }
            for id in &criteria.backdrops {
                snapshot.require_trait(collection, CatalogTrait::Backdrop, id)?;
            }
            for id in &criteria.patterns {
                snapshot.require_trait(collection, CatalogTrait::Pattern, id)?;
            }
        }
        Ok(resolved)
    }
}
impl ResolvedTarget {
    pub fn kind(&self) -> ValuationTargetKind {
        match self {
            Self::Catalog(_) => ValuationTargetKind::Catalog,
            Self::Collection { .. } => ValuationTargetKind::Collection,
            Self::Model { .. } => ValuationTargetKind::Model,
            Self::Backdrop { .. } => ValuationTargetKind::Backdrop,
            Self::ModelBackdrop { .. } => ValuationTargetKind::ModelBackdrop,
            Self::Criteria { .. } => ValuationTargetKind::Criteria,
            Self::Gift { .. } => ValuationTargetKind::Gift,
        }
    }
    pub fn collection(&self) -> Option<&CollectionId> {
        match self {
            Self::Collection { collection_id }
            | Self::Model { collection_id, .. }
            | Self::Backdrop { collection_id, .. }
            | Self::ModelBackdrop { collection_id, .. }
            | Self::Criteria { collection_id, .. } => Some(collection_id),
            Self::Gift { gift, .. } => Some(&gift.collection_id),
            Self::Catalog(_) => None,
        }
    }
    pub fn model(&self) -> Option<&ModelId> {
        match self {
            Self::Model { model_id, .. } | Self::ModelBackdrop { model_id, .. } => Some(model_id),
            _ => None,
        }
    }
    pub fn backdrop(&self) -> Option<&BackdropId> {
        match self {
            Self::Backdrop { backdrop_id, .. } | Self::ModelBackdrop { backdrop_id, .. } => {
                Some(backdrop_id)
            }
            _ => None,
        }
    }
    pub fn gift(&self) -> Option<&CollectibleGift> {
        match self {
            Self::Gift { gift, .. } => Some(gift),
            _ => None,
        }
    }
    pub fn criteria(&self) -> Option<&GiftCriteria> {
        match self {
            Self::Criteria { criteria, .. } => Some(criteria),
            _ => None,
        }
    }
    pub fn slug(&self) -> Option<&GiftSlug> {
        match self {
            Self::Gift { slug, .. } => slug.as_ref(),
            _ => None,
        }
    }
    pub(crate) fn query(
        &self,
        currency: CurrencyId,
        source: OrderSource,
        period: Option<Period>,
    ) -> GiftOrderQuery {
        let mut filters = GiftFilters::new(
            self.collection().expect("single selection").clone(),
            currency,
        );
        if let Some(criteria) = self.criteria() {
            filters.models = criteria.models.clone();
            filters.backdrops = criteria.backdrops.clone();
            filters.patterns = criteria.patterns.clone();
            filters.number = criteria.number.clone();
        }
        filters.models.extend(self.model().cloned());
        filters.backdrops.extend(self.backdrop().cloned());
        if let Some(number) = self.gift().and_then(|gift| gift.number) {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolved_targets_roundtrip_and_reject_missing_or_mixed_payloads() {
        let target = ResolvedTarget::Gift {
            gift: CollectibleGift {
                id: "gift".into(),
                collection_id: "collection".into(),
                number: Some(GiftNumber::new(1).unwrap()),
                traits: GiftTraits::default(),
            },
            slug: None,
        };
        let json = serde_json::to_value(&target).unwrap();
        assert!(json.get("collection_id").is_none());
        assert!(json["gift"].get("collection_id").is_some());
        assert!(matches!(
            serde_json::from_value::<ResolvedTarget>(json).unwrap(),
            ResolvedTarget::Gift { .. }
        ));
        for json in [
            serde_json::json!({"kind":"model","collection_id":"gift"}),
            serde_json::json!({"kind":"gift","collection_id":"gift"}),
            serde_json::json!({"kind":"collection","collection_id":"gift","model_id":"model"}),
            serde_json::json!({"kind":"model_backdrop","collection_id":"gift","model_id":"model"}),
        ] {
            assert!(serde_json::from_value::<ResolvedTarget>(json).is_err());
        }
    }
}
