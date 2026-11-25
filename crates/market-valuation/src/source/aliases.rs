//! Alias resolution uses the same pure catalog adapter inside the valuation's pinned view.
use super::*;
use market_gifts::*;
use market_gifts_hub::input::{CollectionRef, GiftCriteriaInput, GiftFilters};
use market_hub_protocol::wire as w;
impl ValuationSnapshot {
    pub(crate) fn alias_revision(&self) -> Result<market_replica::MetadataRevision> {
        self.reader
            .metadata_revision(&[w::EntityKind::ItemType, w::EntityKind::AttributeOption])
    }
    pub(crate) fn used_aliases(&self) -> bool {
        self.aliases.get().is_some()
    }
    pub(crate) fn alias_catalog(&self) -> Result<Arc<market_gifts_hub::AliasCatalog>> {
        self.aliases
            .get_or_init(|| {
                self.check_cancelled()?;
                let items = market_replica::catalog_read::entities_cancellable(
                    &self.reader,
                    w::EntityKind::ItemType,
                    None,
                    &self.cancelled,
                )?
                .into_iter()
                .filter_map(|v| match v {
                    w::entity::Entity::ItemType(v) => Some(v),
                    _ => None,
                })
                .collect::<Vec<_>>();
                let options = market_replica::catalog_read::entities_cancellable(
                    &self.reader,
                    w::EntityKind::AttributeOption,
                    None,
                    &self.cancelled,
                )?
                .into_iter()
                .filter_map(|v| match v {
                    w::entity::Entity::AttributeOption(v) => Some(v),
                    _ => None,
                })
                .collect::<Vec<_>>();
                market_gifts_hub::AliasCatalog::new_checked(&items, &options, || {
                    self.check_cancelled().map_err(|_| AliasError::Cancelled)
                })
                .map(Arc::new)
                .map_err(alias_status)
            })
            .clone()
    }
    pub(crate) fn resolve_target(
        &self,
        target: &crate::ValuationTarget,
    ) -> Result<crate::ResolvedTarget> {
        use crate::{ResolvedTarget as R, ValuationTarget as T};
        let collection = |id: &CollectionRef| -> Result<CollectionId> {
            match id {
                market_gifts_hub::input::CatalogSelector::Ref(CatalogRef::Id(id)) => Ok(id.clone()),
                _ => self.alias_catalog()?.collection(id).map_err(alias_status),
            }
        };
        let filters =
            |id: &CollectionRef, criteria: &GiftCriteriaInput| -> Result<ResolvedGiftFilters> {
                let input = GiftFilters {
                    collection_id: Some(id.clone()),
                    currency_id: "unused".into(),
                    venue_id: None,
                    models: criteria.models.clone(),
                    backdrops: criteria.backdrops.clone(),
                    patterns: criteria.patterns.clone(),
                    number: criteria.number.clone(),
                    price: None,
                };
                if input.has_aliases() {
                    self.alias_catalog()?.filters(&input).map_err(alias_status)
                } else {
                    input.into_resolved().map_err(alias_status)
                }
            };
        Ok(match target {
            T::Selection {
                collection_id,
                criteria,
            } => {
                let native = filters(collection_id, criteria)?;
                R::Selection {
                    collection_id: native.collection_id.unwrap(),
                    criteria: GiftCriteria {
                        models: native.models,
                        backdrops: native.backdrops,
                        patterns: native.patterns,
                        number: native.number,
                    },
                }
            }
            T::GiftAttributes(gift) => {
                let native = filters(
                    &gift.collection_id,
                    &GiftCriteriaInput {
                        models: gift.traits.model.iter().cloned().collect(),
                        backdrops: gift.traits.backdrop.iter().cloned().collect(),
                        patterns: gift.traits.pattern.iter().cloned().collect(),
                        number: None,
                    },
                )?;
                R::GiftAttributes(crate::GiftAttributes {
                    collection_id: native.collection_id.unwrap(),
                    traits: GiftTraits {
                        model: native.models.into_iter().next(),
                        backdrop: native.backdrops.into_iter().next(),
                        pattern: native.patterns.into_iter().next(),
                    },
                    number: gift.number,
                })
            }
            T::Gift { gift } => {
                let lookup = match gift {
                    GiftLookup::Slug(slug) => GiftLookup::Slug(slug.clone()),
                    GiftLookup::Catalog {
                        collection_id,
                        number,
                    } => GiftLookup::Catalog {
                        collection_id: collection(collection_id)?,
                        number: *number,
                    },
                };
                R::Gift {
                    gift: self.gift(&lookup)?,
                    slug: match gift {
                        GiftLookup::Slug(slug) => Some(slug.clone()),
                        _ => None,
                    },
                }
            }
            T::Catalog(selection) => {
                let collection_ids = selection
                    .collection_ids
                    .iter()
                    .map(collection)
                    .collect::<Result<Vec<_>>>()?;
                let details = selection
                    .details
                    .as_ref()
                    .map(|d| {
                        let pair = d
                            .pair
                            .as_ref()
                            .map(|p| {
                                let native = filters(
                                    &d.collection_id,
                                    &GiftCriteriaInput {
                                        models: vec![p.model_id.clone()],
                                        backdrops: vec![p.backdrop_id.clone()],
                                        ..Default::default()
                                    },
                                )?;
                                Ok::<_, Status>(crate::PairSelection {
                                    model_id: native.models.into_iter().next().unwrap(),
                                    backdrop_id: native.backdrops.into_iter().next().unwrap(),
                                })
                            })
                            .transpose()?;
                        Ok::<_, Status>(crate::CatalogDetails {
                            collection_id: collection(&d.collection_id)?,
                            pair,
                        })
                    })
                    .transpose()?;
                R::Catalog(crate::CatalogSelection {
                    collection_ids,
                    details,
                })
            }
        })
    }
}
fn alias_status(error: AliasError) -> Status {
    match error {
        market_gifts::AliasError::Cancelled => Status::cancelled(error.to_string()),
        market_gifts::AliasError::BudgetExceeded(_) => {
            Status::resource_exhausted(error.to_string())
        }
        market_gifts::AliasError::InvalidCatalog(_) => Status::unavailable(error.to_string()),
        _ => Status::invalid_argument(error.to_string()),
    }
}
