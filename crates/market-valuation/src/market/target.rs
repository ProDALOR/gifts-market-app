use crate::{
    ValuationError,
    source::{GiftTraitKind, ValuationSnapshot},
};
use market_gifts::*;
use market_gifts_hub::input::{
    BackdropRef, CollectionRef, GiftCriteriaInput, GiftLookupInput, GiftTraitsInput, ModelRef,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationTargetKind {
    Catalog,
    Selection,
    Gift,
    GiftAttributes,
}
/// Known gift attributes, without asserting a market listing or a collectible identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftAttributes<C = CollectionId, T = GiftTraits> {
    pub collection_id: C,
    pub traits: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<GiftNumber>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationTarget {
    Catalog(CatalogSelection),
    Selection {
        collection_id: CollectionRef,
        criteria: GiftCriteriaInput,
    },
    Gift {
        gift: GiftLookupInput,
    },
    GiftAttributes(GiftAttributesInput),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CatalogSelection<C = CollectionRef, M = ModelRef, B = BackdropRef> {
    /// One to eight distinct collections in this currency.
    #[cfg_attr(feature = "schema", schema(min_items = 1, max_items = 8))]
    pub collection_ids: Vec<C>,
    /// Include all known models/backdrops and an optional pair for one collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<CatalogDetails<C, M, B>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CatalogDetails<C = CollectionRef, M = ModelRef, B = BackdropRef> {
    pub collection_id: C,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<PairSelection<M, B>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PairSelection<M = ModelRef, B = BackdropRef> {
    pub model_id: M,
    pub backdrop_id: B,
}
pub type ResolvedCatalogSelection = CatalogSelection<CollectionId, ModelId, BackdropId>;
pub type GiftAttributesInput = GiftAttributes<CollectionRef, GiftTraitsInput>;
impl<C: Ord, M, B> CatalogSelection<C, M, B> {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ResolvedTarget {
    Catalog(ResolvedCatalogSelection),
    Selection {
        collection_id: CollectionId,
        criteria: GiftCriteria,
    },
    Gift {
        gift: CollectibleGift,
        #[serde(skip_serializing_if = "Option::is_none")]
        slug: Option<GiftSlug>,
    },
    GiftAttributes(GiftAttributes),
}
impl super::PreparedQuery {
    pub(super) fn resolve_target(
        &self,
        snapshot: &ValuationSnapshot,
    ) -> Result<ResolvedTarget, ValuationError> {
        snapshot.require_currency(&self.currency_id)?;
        if let Some(gift) = &self.external_gift {
            Ok(ResolvedTarget::Gift {
                gift: snapshot.telegram_gift(gift)?,
                slug: Some(gift.slug.clone()),
            })
        } else {
            self.target.resolve(snapshot)
        }
    }
}
impl ValuationTarget {
    pub(crate) fn normalize(&mut self) -> Result<(), ValuationError> {
        match self {
            Self::Catalog(selection) => {
                selection.validate()?;
                selection.collection_ids.sort();
            }
            Self::Selection { criteria, .. } => {
                if criteria.models.len() + criteria.backdrops.len() + criteria.patterns.len() > 500
                {
                    return Err(ValuationError::invalid(
                        "at most 500 accepted trait alternatives",
                    ));
                }
                if let Some(number) = &criteria.number {
                    number
                        .validate()
                        .map_err(|e| ValuationError::invalid(e.to_string()))?;
                }
                criteria.models.sort();
                criteria.models.dedup();
                criteria.backdrops.sort();
                criteria.backdrops.dedup();
                criteria.patterns.sort();
                criteria.patterns.dedup();
            }
            Self::Gift { .. } | Self::GiftAttributes(_) => {}
        }
        Ok(())
    }
    pub(crate) fn resolve(
        &self,
        snapshot: &ValuationSnapshot,
    ) -> Result<ResolvedTarget, ValuationError> {
        let target = snapshot.resolve_target(self)?;
        match &target {
            ResolvedTarget::Catalog(selection) => {
                selection.validate()?;
                for collection in &selection.collection_ids {
                    snapshot.require_collection(collection)?;
                }
            }
            ResolvedTarget::Selection {
                collection_id,
                criteria,
            } => {
                snapshot.require_collection(collection_id)?;
                for id in &criteria.models {
                    snapshot.require_trait(collection_id, GiftTraitKind::Model, id)?;
                }
                for id in &criteria.backdrops {
                    snapshot.require_trait(collection_id, GiftTraitKind::Backdrop, id)?;
                }
                for id in &criteria.patterns {
                    snapshot.require_trait(collection_id, GiftTraitKind::Pattern, id)?;
                }
            }
            ResolvedTarget::GiftAttributes(attributes) => {
                snapshot.require_collection(&attributes.collection_id)?;
                for (kind, id) in [
                    (
                        GiftTraitKind::Model,
                        attributes.traits.model.as_ref().map(ModelId::as_str),
                    ),
                    (
                        GiftTraitKind::Backdrop,
                        attributes.traits.backdrop.as_ref().map(BackdropId::as_str),
                    ),
                    (
                        GiftTraitKind::Pattern,
                        attributes.traits.pattern.as_ref().map(PatternId::as_str),
                    ),
                ] {
                    if let Some(id) = id {
                        snapshot.require_trait(&attributes.collection_id, kind, id)?;
                    }
                }
            }
            ResolvedTarget::Gift { .. } => {}
        }
        Ok(target)
    }
}
impl ResolvedTarget {
    pub fn kind(&self) -> ValuationTargetKind {
        match self {
            Self::Catalog(_) => ValuationTargetKind::Catalog,
            Self::Selection { .. } => ValuationTargetKind::Selection,
            Self::Gift { .. } => ValuationTargetKind::Gift,
            Self::GiftAttributes(_) => ValuationTargetKind::GiftAttributes,
        }
    }
    pub fn collection(&self) -> Option<&CollectionId> {
        match self {
            Self::Selection { collection_id, .. } => Some(collection_id),
            Self::Gift { gift, .. } => Some(&gift.collection_id),
            Self::GiftAttributes(attributes) => Some(&attributes.collection_id),
            Self::Catalog(_) => None,
        }
    }
    pub fn criteria(&self) -> Option<&GiftCriteria> {
        match self {
            Self::Selection { criteria, .. } => Some(criteria),
            _ => None,
        }
    }
    pub fn model(&self) -> Option<&ModelId> {
        if let Self::GiftAttributes(attributes) = self {
            return attributes.traits.model.as_ref();
        }
        self.criteria()
            .and_then(|criteria| (criteria.models.len() == 1).then(|| &criteria.models[0]))
    }
    pub fn backdrop(&self) -> Option<&BackdropId> {
        if let Self::GiftAttributes(attributes) = self {
            return attributes.traits.backdrop.as_ref();
        }
        self.criteria()
            .and_then(|criteria| (criteria.backdrops.len() == 1).then(|| &criteria.backdrops[0]))
    }
    pub fn gift(&self) -> Option<&CollectibleGift> {
        match self {
            Self::Gift { gift, .. } => Some(gift),
            _ => None,
        }
    }
    pub fn pattern(&self) -> Option<&PatternId> {
        if let Self::GiftAttributes(attributes) = self {
            return attributes.traits.pattern.as_ref();
        }
        self.criteria()
            .and_then(|criteria| (criteria.patterns.len() == 1).then(|| &criteria.patterns[0]))
    }
    pub fn slug(&self) -> Option<&GiftSlug> {
        match self {
            Self::Gift { slug, .. } => slug.as_ref(),
            _ => None,
        }
    }
    /// Singleton model/backdrop selections reuse the same bounded statistics as catalog floors.
    pub(crate) fn simple_floor(&self) -> bool {
        self.criteria().is_some_and(|criteria| {
            criteria.models.len() <= 1
                && criteria.backdrops.len() <= 1
                && (criteria.models.is_empty() || criteria.backdrops.is_empty())
                && criteria.patterns.is_empty()
                && criteria.number.is_none()
        })
    }
    pub(crate) fn query(
        &self,
        currency: CurrencyId,
        source: OrderSource,
        period: Option<Period>,
    ) -> ResolvedGiftOrderQuery {
        let mut filters = ResolvedGiftFilters::new(
            self.collection().expect("single selection").clone(),
            currency,
        );
        if let Some(criteria) = self.criteria() {
            filters.models = criteria.models.clone();
            filters.backdrops = criteria.backdrops.clone();
            filters.patterns = criteria.patterns.clone();
            filters.number = criteria.number.clone();
        }
        if let Self::GiftAttributes(attributes) = self {
            filters
                .models
                .extend(attributes.traits.model.iter().cloned());
            filters
                .backdrops
                .extend(attributes.traits.backdrop.iter().cloned());
            filters
                .patterns
                .extend(attributes.traits.pattern.iter().cloned());
            filters.number = attributes.number.map(|number| GiftNumberRange {
                min: Some(number),
                max: Some(number),
            });
        }
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
    fn selection_is_canonical_and_legacy_targets_are_rejected() {
        let mut target: ValuationTarget = serde_json::from_value(serde_json::json!({"kind":"selection","collection_id":"gift","criteria":{"models":["b","a","b"],"patterns":["p","p"]}})).unwrap();
        target.normalize().unwrap();
        let ValuationTarget::Selection { criteria, .. } = target else {
            panic!()
        };
        assert_eq!(
            criteria.models,
            vec![ModelRef::from("a"), ModelRef::from("b")]
        );
        assert_eq!(criteria.patterns.len(), 1);
        for kind in [
            "collection",
            "model",
            "backdrop",
            "model_backdrop",
            "criteria",
        ] {
            assert!(
                serde_json::from_value::<ValuationTarget>(
                    serde_json::json!({"kind":kind,"collection_id":"gift","criteria":{}})
                )
                .is_err()
            );
        }
    }
}

#[cfg(test)]
mod typed_targets {
    use super::*;
    #[test]
    fn input_targets_keep_the_existing_json_but_resolved_targets_cannot_contain_aliases() {
        for input in [
            serde_json::json!({"kind":"selection","collection_id":"@tg.slug:One","criteria":{"models":["@name:Model"],"backdrops":[],"patterns":[]}}),
            serde_json::json!({"kind":"gift_attributes","collection_id":"@tg.slug:One","traits":{"model":"@tg:123","backdrop":"@name:Black","pattern":"@tg:42"}}),
            serde_json::json!({"kind":"catalog","collection_ids":["@tg.slug:One"],"details":{"collection_id":"@tg.slug:One","pair":{"model_id":"@name:Model","backdrop_id":"@tg:42"}}}),
            serde_json::json!({"kind":"gift","gift":{"collection_id":"@tg.slug:One","number":7}}),
        ] {
            let target: ValuationTarget = serde_json::from_value(input.clone()).unwrap();
            assert_eq!(serde_json::to_value(&target).unwrap(), input);
            assert!(serde_json::from_value::<ResolvedTarget>(input).is_err());
        }
    }
}
