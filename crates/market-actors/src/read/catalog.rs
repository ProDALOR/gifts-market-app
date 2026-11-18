use super::*;
use market_gifts::{valuation::ModelLookup, *};
use market_hub_protocol::wire as w;
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug)]
pub enum CatalogTrait {
    Model,
    Backdrop,
}
impl CatalogTrait {
    pub(super) fn attribute(self) -> &'static str {
        match self {
            Self::Model => market_gifts_hub::MODEL,
            Self::Backdrop => market_gifts_hub::BACKDROP,
        }
    }
}
impl MarketSnapshot {
    pub fn resolve_models(&self, lookups: Vec<ModelLookup>) -> Result<Vec<ResolvedModel>> {
        self.check_cancelled()?;
        Ok(crate::catalog::resolve_models(
            &self.reader,
            lookups,
            &self.cancelled,
        )?)
    }
    pub(super) fn entity(
        &self,
        kind: w::EntityKind,
        id: &str,
    ) -> Result<Option<w::entity::Entity>> {
        self.check_cancelled()?;
        Ok(crate::catalog::projection::entities_cancellable(
            &self.reader,
            kind,
            Some(&[id.to_owned()]),
            &self.cancelled,
        )?
        .into_iter()
        .next())
    }
    pub fn require_currency(&self, currency: &CurrencyId) -> Result<()> {
        self.entity(w::EntityKind::Currency, currency)?
            .ok_or_else(|| DataError::invalid("unknown quote currency"))?;
        Ok(())
    }
    fn options(&self, collection: &CollectionId, kind: CatalogTrait) -> Result<Vec<String>> {
        let Some(w::entity::Entity::ItemType(item)) =
            self.entity(w::EntityKind::ItemType, collection)?
        else {
            return Err(DataError::invalid("unknown gift collection"));
        };
        if !market_gifts_hub::is_gift(&item) {
            return Err(DataError::invalid("unknown gift collection"));
        }
        let rules = market_gifts_hub::CollectionChoices::new(&item)
            .map_err(|e| DataError::unavailable(e.to_string()))?;
        let ids = rules.trait_option_ids(kind.attribute());
        let options = crate::catalog::projection::trait_options(
            &self.reader,
            kind.attribute(),
            ids.as_deref(),
            &self.cancelled,
        )?;
        Ok(options
            .into_iter()
            .filter(|option| rules.accepts(option))
            .map(|option| option.id)
            .collect())
    }
    pub fn model_ids(&self, collection: &CollectionId) -> Result<BTreeSet<ModelId>> {
        self.options(collection, CatalogTrait::Model)?
            .into_iter()
            .map(|id| {
                id.try_into()
                    .map_err(|e| DataError::unavailable(format!("{e}")))
            })
            .collect()
    }
    pub fn backdrop_ids(&self, collection: &CollectionId) -> Result<BTreeSet<BackdropId>> {
        self.options(collection, CatalogTrait::Backdrop)?
            .into_iter()
            .map(|id| {
                id.try_into()
                    .map_err(|e| DataError::unavailable(format!("{e}")))
            })
            .collect()
    }
}
