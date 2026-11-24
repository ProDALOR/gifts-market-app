use super::*;
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug)]
pub enum CatalogTrait {
    Model,
    Backdrop,
    Pattern,
}
impl CatalogTrait {
    pub(super) fn attribute(self) -> &'static str {
        match self {
            Self::Model => market_gifts_hub::MODEL,
            Self::Backdrop => market_gifts_hub::BACKDROP,
            Self::Pattern => market_gifts_hub::PATTERN,
        }
    }
}
impl ValuationSnapshot {
    pub fn require_collection(&self, collection: &CollectionId) -> Result<()> {
        let item = self.entity(w::EntityKind::ItemType, collection)?;
        if matches!(item, Some(w::entity::Entity::ItemType(ref item)) if market_gifts_hub::is_gift(item))
        {
            Ok(())
        } else {
            Err(Status::invalid_argument("unknown gift collection"))
        }
    }
    /// Validate one selected trait with direct catalog keys, without loading the trait directory.
    pub fn require_trait(
        &self,
        collection: &CollectionId,
        kind: CatalogTrait,
        id: &str,
    ) -> Result<()> {
        let Some(w::entity::Entity::ItemType(item)) =
            self.entity(w::EntityKind::ItemType, collection)?
        else {
            return Err(Status::invalid_argument("unknown gift collection"));
        };
        let rules = market_gifts_hub::CollectionChoices::new(&item)
            .map_err(|e| Status::unavailable(e.to_string()))?;
        if let Some(w::entity::Entity::AttributeOption(option)) =
            self.entity(w::EntityKind::AttributeOption, id)?
            && option.attribute_id == kind.attribute()
            && rules.accepts(&option)
        {
            return Ok(());
        }
        Err(Status::invalid_argument(
            "trait does not belong to the selected collection or kind",
        ))
    }
    pub(super) fn entity(
        &self,
        kind: w::EntityKind,
        id: &str,
    ) -> Result<Option<w::entity::Entity>> {
        self.check_cancelled()?;
        Ok(market_replica::catalog_read::entities_cancellable(
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
            .ok_or_else(|| Status::invalid_argument("unknown quote currency"))?;
        Ok(())
    }
    fn options(&self, collection: &CollectionId, kind: CatalogTrait) -> Result<Vec<String>> {
        let Some(w::entity::Entity::ItemType(item)) =
            self.entity(w::EntityKind::ItemType, collection)?
        else {
            return Err(Status::invalid_argument("unknown gift collection"));
        };
        if !market_gifts_hub::is_gift(&item) {
            return Err(Status::invalid_argument("unknown gift collection"));
        }
        let rules = market_gifts_hub::CollectionChoices::new(&item)
            .map_err(|e| Status::unavailable(e.to_string()))?;
        let ids = rules.trait_option_ids(kind.attribute());
        let options = market_replica::catalog_read::trait_options(
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
                    .map_err(|e| Status::unavailable(format!("{e}")))
            })
            .collect()
    }
    pub fn backdrop_ids(&self, collection: &CollectionId) -> Result<BTreeSet<BackdropId>> {
        self.options(collection, CatalogTrait::Backdrop)?
            .into_iter()
            .map(|id| {
                id.try_into()
                    .map_err(|e| Status::unavailable(format!("{e}")))
            })
            .collect()
    }
    pub fn pattern_ids(&self, collection: &CollectionId) -> Result<BTreeSet<PatternId>> {
        self.options(collection, CatalogTrait::Pattern)?
            .into_iter()
            .map(|id| {
                id.try_into()
                    .map_err(|e| Status::unavailable(format!("{e}")))
            })
            .collect()
    }
}
