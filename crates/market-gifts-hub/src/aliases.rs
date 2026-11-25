//! One pure resolver for native IDs, explicit source aliases and scoped name lookups.
use crate::input::{CatalogSelector, CollectionRef, GiftFilters, SourceAlias};
use crate::{BACKDROP, CollectionChoices, PATTERN};
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::collections::{BTreeMap, BTreeSet};

pub fn aliases(values: &[w::ExternalId]) -> Vec<GiftAlias> {
    let mut result: Vec<_> = values
        .iter()
        .filter_map(|a| match a.namespace.as_str() {
            "tg.slug" | "slug" => GiftAlias::new("slug", a.id.clone()).ok(),
            "name" => GiftAlias::new("name", a.id.clone()).ok(),
            _ => None,
        })
        .collect();
    result.sort();
    result.dedup();
    result
}
fn source_aliases(values: &[w::ExternalId]) -> Vec<SourceAlias> {
    values
        .iter()
        .filter_map(|a| SourceAlias::new(&a.namespace, &a.id).ok())
        .collect()
}
#[derive(Clone)]
struct Entry {
    id: String,
    name: String,
    aliases: Vec<SourceAlias>,
}
impl Entry {
    fn matches(&self, selector: &CollectionRef) -> bool {
        if let Some(id) = selector.as_id() {
            return self.id == id.as_str();
        }
        if let CatalogSelector::Ref(CatalogRef::Name(name)) = selector
            && self.name.eq_ignore_ascii_case(name.as_str())
        {
            return true;
        }
        let key = selector.lookup_key();
        self.aliases.iter().any(|alias| alias.lookup_key() == key)
    }
}
pub struct AliasCatalog {
    collections: BTreeMap<String, (Entry, CollectionChoices)>,
    conflicts: BTreeSet<String>,
    lookup: BTreeMap<(GiftTraitKind, String), BTreeMap<String, BTreeSet<String>>>,
    pub backdrops: std::sync::Arc<Vec<GiftBackdrop>>,
    pub patterns: std::sync::Arc<Vec<GiftPattern>>,
}
impl AliasCatalog {
    pub fn new(items: &[w::ItemType], options: &[w::AttributeOption]) -> Result<Self, AliasError> {
        Self::new_checked(items, options, || Ok(()))
    }
    pub fn new_checked(
        items: &[w::ItemType],
        options: &[w::AttributeOption],
        mut check: impl FnMut() -> Result<(), AliasError>,
    ) -> Result<Self, AliasError> {
        check()?;
        let collections = items
            .iter()
            .filter(|v| crate::is_gift(v))
            .map(|v| {
                Ok((
                    v.id.clone(),
                    (
                        Entry {
                            id: v.id.clone(),
                            name: v.name.clone(),
                            aliases: source_aliases(&v.aliases),
                        },
                        CollectionChoices::new(v).map_err(|_| {
                            AliasError::InvalidCatalog("invalid collection choices")
                        })?,
                    ),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, AliasError>>()?;
        let by_id: BTreeMap<_, _> = options.iter().map(|v| (&v.id, v)).collect();
        let mut lookup: BTreeMap<(GiftTraitKind, String), BTreeMap<String, BTreeSet<String>>> =
            BTreeMap::new();
        let mut memberships = 0usize;
        let mut links = 0usize;
        for (collection, (_, rules)) in &collections {
            check()?;
            for kind in [
                GiftTraitKind::Model,
                GiftTraitKind::Backdrop,
                GiftTraitKind::Pattern,
            ] {
                let attribute = crate::trait_attribute(kind);
                let allowed = rules.trait_option_ids(kind);
                let candidates: Box<dyn Iterator<Item = &w::AttributeOption> + '_> =
                    if let Some(ref ids) = allowed {
                        Box::new(ids.iter().filter_map(|id| by_id.get(id).copied()))
                    } else {
                        Box::new(options.iter().filter(|v| v.attribute_id == attribute))
                    };
                for option in candidates {
                    check()?;
                    memberships += 1;
                    if memberships > 200_000 {
                        return Err(AliasError::BudgetExceeded(
                            "catalog alias membership budget exceeded",
                        ));
                    }
                    if option.attribute_id != attribute {
                        continue;
                    }
                    let selectors = std::iter::once(option.id.clone())
                        .chain(
                            GiftAlias::new("name", &option.name)
                                .ok()
                                .into_iter()
                                .map(|v| v.lookup_key()),
                        )
                        .chain(
                            source_aliases(&option.aliases)
                                .into_iter()
                                .map(|v| v.lookup_key()),
                        );
                    for selector in selectors {
                        links += 1;
                        if links > 1_000_000 {
                            return Err(AliasError::BudgetExceeded(
                                "catalog alias index budget exceeded",
                            ));
                        }
                        lookup
                            .entry((kind, selector))
                            .or_default()
                            .entry(collection.clone())
                            .or_default()
                            .insert(option.id.clone());
                    }
                }
            }
        }
        let mut palettes = BTreeMap::new();
        let mut conflicts = BTreeSet::new();
        for option in options.iter().filter(|o| o.attribute_id == BACKDROP) {
            check()?;
            if let Some(palette) = crate::palette(option) {
                for key in source_aliases(&option.aliases)
                    .into_iter()
                    .map(|a| a.lookup_key())
                    .chain(
                        GiftAlias::new("name", option.name.clone())
                            .ok()
                            .map(|a| a.lookup_key()),
                    )
                {
                    if palettes
                        .get(&key)
                        .is_some_and(|previous| previous != &palette)
                    {
                        conflicts.insert(key.clone());
                    }
                    palettes.insert(key, palette.clone());
                }
            }
        }
        let mut result = Self {
            collections,
            conflicts,
            lookup,
            backdrops: Default::default(),
            patterns: Default::default(),
        };
        let mut seen = BTreeSet::new();
        for option in options {
            check()?;
            if !matches!(option.attribute_id.as_str(), BACKDROP | PATTERN)
                || !result
                    .collections
                    .values()
                    .any(|(_, rules)| rules.accepts(option))
            {
                continue;
            }
            // Source identity, never a mutable display name, groups shared picker choices.
            let kind = crate::trait_kind(&option.attribute_id).expect("gift trait");
            let name = GiftAlias::new("name", option.name.clone()).ok();
            let name_entries = name
                .as_ref()
                .and_then(|name| result.lookup.get(&(kind, name.lookup_key())));
            let source = source_aliases(&option.aliases)
                .into_iter()
                .find(|a| a.namespace() == "tg" && !result.conflicts.contains(&a.lookup_key()));
            let safe = |alias: &GiftAlias| {
                !result.conflicts.contains(&alias.lookup_key())
                    && result
                        .lookup
                        .get(&(kind, alias.lookup_key()))
                        .is_some_and(|entries| entries.values().all(|ids| ids.len() == 1))
            };
            let public_alias = aliases(&option.aliases)
                .into_iter()
                .find(|alias| safe(alias))
                .or_else(|| {
                    name.filter(|name| {
                        safe(name)
                            && name_entries.is_some_and(|entries| {
                                source
                                    .as_ref()
                                    .and_then(|a| result.lookup.get(&(kind, a.lookup_key())))
                                    .is_some_and(|source| source == entries)
                            })
                    })
                });
            let key = public_alias
                .as_ref()
                .map(GiftAlias::lookup_key)
                .unwrap_or_else(|| option.id.clone());
            if !seen.insert((option.attribute_id.clone(), key)) {
                continue;
            }
            match crate::choice(option.clone())
                .map_err(|_| AliasError::InvalidCatalog("invalid catalog choice"))?
            {
                Some(crate::Choice::Backdrop(mut v)) => {
                    v.rarity = None;
                    v.aliases = public_alias.clone().into_iter().collect();
                    std::sync::Arc::make_mut(&mut result.backdrops).push(v);
                }
                Some(crate::Choice::Pattern(mut v)) => {
                    v.rarity = None;
                    v.aliases = public_alias.into_iter().collect();
                    std::sync::Arc::make_mut(&mut result.patterns).push(v);
                }
                _ => {}
            }
        }
        std::sync::Arc::make_mut(&mut result.backdrops)
            .sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
        std::sync::Arc::make_mut(&mut result.patterns)
            .sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
        Ok(result)
    }
    pub fn contains_collection(&self, id: &str) -> bool {
        self.collections.contains_key(id)
    }
    pub fn collection(&self, selector: &CollectionRef) -> Result<CollectionId, AliasError> {
        self.collection_if_known(selector)?
            .ok_or(AliasError::UnknownCollection)
    }
    pub fn collection_if_known(
        &self,
        selector: &CollectionRef,
    ) -> Result<Option<CollectionId>, AliasError> {
        if let Some(id) = selector.as_id() {
            return Ok(self
                .collections
                .contains_key(id.as_str())
                .then(|| id.clone()));
        }
        let mut found = self
            .collections
            .values()
            .filter(|(entry, _)| entry.matches(selector));
        let Some((first, _)) = found.next() else {
            return Ok(None);
        };
        if found.next().is_some() {
            return Err(AliasError::AmbiguousCollection);
        }
        CollectionId::new(first.id.clone())
            .map(Some)
            .map_err(|_| AliasError::InvalidCatalog("invalid collection identity"))
    }
    fn options<I: std::fmt::Display>(
        &self,
        kind: GiftTraitKind,
        collection: Option<&CollectionId>,
        selectors: &[CatalogSelector<I>],
    ) -> Result<Vec<String>, AliasError> {
        if selectors.is_empty() {
            return Ok(vec![]);
        }
        if kind == GiftTraitKind::Model && collection.is_none() {
            return Err(AliasError::ModelRequiresCollection);
        }
        let mut result = BTreeSet::new();
        for selector in selectors {
            let key = (kind, selector.lookup_key());
            if kind == GiftTraitKind::Backdrop
                && collection.is_none()
                && self.conflicts.contains(&key.1)
            {
                return Err(AliasError::ConflictingBackdrop);
            }
            let entries = self
                .lookup
                .get(&key)
                .ok_or(AliasError::UnknownTrait(kind))?;
            let mut matched = false;
            for (id, values) in entries {
                if collection.is_some_and(|c| c.as_str() != id) {
                    continue;
                }
                if values.len() > 1 {
                    return Err(AliasError::AmbiguousTrait(kind));
                }
                matched = true;
                result.extend(values.iter().cloned());
            }
            if !matched {
                return Err(AliasError::UnknownTrait(kind));
            }
            if result.len() > 4096 {
                return Err(AliasError::BudgetExceeded(
                    "alias expansion exceeds trait budget",
                ));
            }
        }
        Ok(result.into_iter().collect())
    }
    pub fn filters(&self, filters: &GiftFilters) -> Result<ResolvedGiftFilters, AliasError> {
        let collection = filters
            .collection_id
            .as_ref()
            .map(|c| self.collection(c))
            .transpose()?;
        Ok(ResolvedGiftFilters {
            collection_id: collection.clone(),
            currency_id: filters.currency_id.clone(),
            venue_id: filters.venue_id.clone(),
            models: self
                .options(GiftTraitKind::Model, collection.as_ref(), &filters.models)?
                .into_iter()
                .map(ModelId::try_from)
                .collect::<Result<_, _>>()
                .map_err(|_| AliasError::InvalidCatalog("invalid trait identity"))?,
            backdrops: self
                .options(
                    GiftTraitKind::Backdrop,
                    collection.as_ref(),
                    &filters.backdrops,
                )?
                .into_iter()
                .map(BackdropId::try_from)
                .collect::<Result<_, _>>()
                .map_err(|_| AliasError::InvalidCatalog("invalid trait identity"))?,
            patterns: self
                .options(
                    GiftTraitKind::Pattern,
                    collection.as_ref(),
                    &filters.patterns,
                )?
                .into_iter()
                .map(PatternId::try_from)
                .collect::<Result<_, _>>()
                .map_err(|_| AliasError::InvalidCatalog("invalid trait identity"))?,
            number: filters.number.clone(),
            price: filters.price.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MODEL;
    fn fixture() -> (Vec<w::ItemType>, Vec<w::AttributeOption>) {
        let mut items = Vec::new();
        let mut options = Vec::new();
        for (suffix, slug) in [("1", "One"), ("2", "Two")] {
            let mut attributes = BTreeMap::new();
            for (kind, prefix, name, alias) in [
                (MODEL, "m", "Dogfather", "101"),
                (BACKDROP, "b", "Black", "42"),
                (PATTERN, "p", "Crown", "42"),
            ] {
                let id = format!("{prefix}{suffix}");
                attributes.insert(
                    kind.into(),
                    w::AttributeRule {
                        allowed_values: Some(w::AllowedAttributeValues {
                            ids: vec![id.clone()],
                        }),
                        ..Default::default()
                    },
                );
                options.push(w::AttributeOption {
                    id,
                    name: name.into(),
                    attribute_id: kind.into(),
                    aliases: vec![w::ExternalId {
                        namespace: "tg".into(),
                        id: alias.into(),
                    }],
                    ..Default::default()
                });
            }
            items.push(w::ItemType {
                id: format!("c{suffix}"),
                name: slug.into(),
                attributes,
                aliases: vec![w::ExternalId {
                    namespace: "tg.slug".into(),
                    id: slug.into(),
                }],
                ..Default::default()
            });
        }
        (items, options)
    }
    #[test]
    fn aliases_are_scoped_by_kind_and_models_require_a_collection() {
        let (items, options) = fixture();
        let catalog = AliasCatalog::new(&items, &options).unwrap();
        let mut filters = GiftFilters::new("@tg.slug:oNe".into(), "GRAM".into());
        filters.models = vec!["@name:dogfather".into()];
        filters.backdrops = vec!["@tg:42".into()];
        filters.patterns = vec!["@tg:42".into()];
        let native = catalog.filters(&filters).unwrap();
        assert_eq!(native.collection_id.unwrap().as_str(), "c1");
        assert_eq!(native.models[0].as_str(), "m1");
        assert_eq!(native.backdrops[0].as_str(), "b1");
        assert_eq!(native.patterns[0].as_str(), "p1");
        filters.collection_id = None;
        assert!(
            catalog
                .filters(&filters)
                .unwrap_err()
                .to_string()
                .contains("require a collection")
        );
        filters.models.clear();
        let native = catalog.filters(&filters).unwrap();
        assert_eq!(
            native
                .backdrops
                .iter()
                .map(|v| v.as_str())
                .collect::<Vec<_>>(),
            vec!["b1", "b2"]
        );
        assert_eq!(
            native
                .patterns
                .iter()
                .map(|v| v.as_str())
                .collect::<Vec<_>>(),
            vec!["p1", "p2"]
        );
        assert_eq!(catalog.backdrops.len(), 1);
        assert_eq!(catalog.patterns.len(), 1);
        assert!(catalog.backdrops[0].rarity.is_none());
        assert_eq!(
            catalog
                .filters(&crate::input::filters(native.clone()))
                .unwrap(),
            native,
            "native IDs remain a valid resolved boundary"
        );
    }
    #[test]
    fn canonical_slugs_and_names_resolve_without_exposing_source_namespaces() {
        let (items, mut options) = fixture();
        for option in &mut options {
            if option.attribute_id == BACKDROP {
                option.aliases.push(w::ExternalId {
                    namespace: "slug".into(),
                    id: "Noir".into(),
                });
            }
        }
        let catalog = AliasCatalog::new(&items, &options).unwrap();
        assert_eq!(
            catalog.collection(&"@slug:oNe".into()).unwrap(),
            CollectionId::from("c1")
        );
        let mut filters = GiftFilters::new("@slug:One".into(), "GRAM".into());
        filters.models = vec!["@name:dogfather".into()];
        filters.backdrops = vec!["@slug:NOIR".into()];
        assert_eq!(
            catalog.filters(&filters).unwrap().models,
            vec![ModelId::from("m1")]
        );
        filters.collection_id = None;
        filters.models.clear();
        assert_eq!(catalog.filters(&filters).unwrap().backdrops.len(), 2);
        assert_eq!(catalog.backdrops.len(), 1);
        assert_eq!(
            catalog.backdrops[0].aliases[0],
            GiftAlias::new("slug", "Noir").unwrap()
        );
        let projected = aliases(&items[0].aliases);
        assert_eq!(projected, vec![GiftAlias::new("slug", "One").unwrap()]);
    }
    #[test]
    fn global_picker_does_not_merge_different_source_identities_with_the_same_name() {
        let (items, mut options) = fixture();
        options.iter_mut().find(|v| v.id == "p2").unwrap().aliases[0].id =
            "different-symbol".into();
        let catalog = AliasCatalog::new(&items, &options).unwrap();
        assert_eq!(catalog.patterns.len(), 2);
        assert!(catalog.patterns.iter().all(|p| p.aliases.is_empty()));
    }
    #[test]
    fn duplicates_within_a_collection_are_ambiguous_and_unknown_aliases_never_widen_filters() {
        let (mut items, mut options) = fixture();
        items[0]
            .attributes
            .get_mut(MODEL)
            .unwrap()
            .allowed_values
            .as_mut()
            .unwrap()
            .ids
            .push("duplicate".into());
        let mut duplicate = options[0].clone();
        duplicate.id = "duplicate".into();
        options.push(duplicate);
        let catalog = AliasCatalog::new(&items, &options).unwrap();
        let mut filters = GiftFilters::new("c1".into(), "GRAM".into());
        filters.models = vec!["@tg:101".into()];
        assert!(
            catalog
                .filters(&filters)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        filters.models = vec!["m1".into()];
        assert!(catalog.filters(&filters).is_ok());
        filters.models.clear();
        filters.backdrops = vec!["@tg:missing".into()];
        assert!(catalog.filters(&filters).is_err());
        assert!(BackdropRef::new("@malformed").is_err());
        items[1].aliases = items[0].aliases.clone();
        let catalog = AliasCatalog::new(&items, &options).unwrap();
        assert!(catalog.collection(&"@tg.slug:One".into()).is_err());
    }
    #[test]
    fn catalog_preparation_honors_cancellation_before_building_shared_indexes() {
        let (items, options) = fixture();
        let error = AliasCatalog::new_checked(&items, &options, || Err(AliasError::Cancelled))
            .err()
            .unwrap();
        assert_eq!(error, AliasError::Cancelled);
    }
    #[test]
    fn conflicting_backdrop_palettes_keep_global_choices_separate_and_scoped_aliases_valid() {
        let (items, mut options) = fixture();
        for option in &mut options {
            if option.attribute_id == BACKDROP {
                for key in [
                    "colors.center_color",
                    "colors.edge_color",
                    "colors.pattern_color",
                ] {
                    option.meta.insert(
                        key.into(),
                        if option.id == "b1" {
                            "#000000"
                        } else {
                            "#ffffff"
                        }
                        .into(),
                    );
                }
            }
        }
        let catalog = AliasCatalog::new(&items, &options).unwrap();
        assert_eq!(catalog.backdrops.len(), 2);
        assert!(catalog.backdrops.iter().all(|v| v.aliases.is_empty()));
        let mut filters = GiftFilters::new("c1".into(), "GRAM".into());
        filters.backdrops = vec!["@tg:42".into()];
        assert!(catalog.filters(&filters).is_ok());
        filters.collection_id = None;
        assert!(catalog.filters(&filters).is_err());
        filters.backdrops = vec!["@name:Black".into()];
        assert_eq!(
            catalog.filters(&filters),
            Err(AliasError::ConflictingBackdrop)
        );
        filters.backdrops = vec!["b1".into()];
        assert!(catalog.filters(&filters).is_ok());
    }
}

#[cfg(test)]
mod typed_errors {
    use super::*;
    #[test]
    fn unknown_collections_and_traits_are_distinct_from_ambiguous_aliases() {
        let catalog = AliasCatalog::new(&[], &[]).unwrap();
        assert_eq!(
            catalog.collection(&"@name:Missing".into()),
            Err(AliasError::UnknownCollection)
        );
        let mut filters: GiftFilters = GiftFilters::new("unused".into(), "GRAM".into());
        filters.collection_id = None;
        filters.patterns = vec!["@tg:42".into()];
        assert_eq!(
            catalog.filters(&filters),
            Err(AliasError::UnknownTrait(GiftTraitKind::Pattern))
        );
        filters.models = vec!["@name:Model".into()];
        assert_eq!(
            catalog.filters(&filters),
            Err(AliasError::ModelRequiresCollection)
        );
    }
    #[test]
    fn trait_kinds_translate_only_at_the_generic_hub_boundary() {
        for kind in [
            GiftTraitKind::Model,
            GiftTraitKind::Backdrop,
            GiftTraitKind::Pattern,
        ] {
            assert_eq!(crate::trait_kind(crate::trait_attribute(kind)), Some(kind));
        }
        assert_eq!(crate::trait_kind("number"), None);
        assert_eq!(crate::trait_kind("tg.number"), None);
    }
}
