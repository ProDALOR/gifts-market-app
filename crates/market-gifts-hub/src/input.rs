//! Compatibility at the hub adapter boundary. Source identities never become domain slugs.
use market_gifts::{
    AliasError, BackdropId, CatalogRef, CatalogReference, CollectionId, InvalidValue, ModelId,
    PatternId,
};
use serde::{Deserialize, Serialize};
use std::fmt;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceAlias {
    namespace: String,
    id: String,
}
impl SourceAlias {
    pub fn new(namespace: impl Into<String>, id: impl Into<String>) -> Result<Self, InvalidValue> {
        let namespace = namespace.into();
        let id = id.into();
        if namespace.is_empty()
            || namespace.len() > 64
            || !namespace
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || id.is_empty()
            || namespace.len() + id.len() + 2 > 256
            || id.trim() != id
            || id.chars().any(char::is_control)
        {
            return Err(InvalidValue("invalid source alias"));
        }
        Ok(Self { namespace, id })
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn selector(&self) -> String {
        format!("@{}:{}", self.namespace, self.id)
    }
    pub fn lookup_key(&self) -> String {
        match self.namespace.as_str() {
            "tg.slug" | "slug" => format!("@slug:{}", self.id.to_ascii_lowercase()),
            "name" => format!("@name:{}", self.id.to_ascii_lowercase()),
            _ => self.selector(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CatalogSelector<I> {
    Ref(CatalogRef<I>),
    Source(SourceAlias),
}
pub type CollectionRef = CatalogSelector<CollectionId>;
pub type ModelRef = CatalogSelector<ModelId>;
pub type BackdropRef = CatalogSelector<BackdropId>;
pub type PatternRef = CatalogSelector<PatternId>;
pub type GiftFilters = market_gifts::GiftFilters<CollectionRef, ModelRef, BackdropRef, PatternRef>;
pub type GiftOrderQuery = market_gifts::GiftOrderQuery<GiftFilters>;
pub type GiftCriteriaInput = market_gifts::GiftCriteria<ModelRef, BackdropRef, PatternRef>;
pub type GiftTraitsInput = market_gifts::GiftTraits<ModelRef, BackdropRef, PatternRef>;
pub type GiftLookupInput = market_gifts::GiftLookup<CollectionRef>;
pub type CatalogVersions = market_gifts::CatalogVersions<CollectionRef>;
impl<I> CatalogReference for CatalogSelector<I> {
    type Id = I;
    fn as_id(&self) -> Option<&I> {
        match self {
            Self::Ref(v) => v.as_id(),
            Self::Source(_) => None,
        }
    }
    fn into_id(self) -> Result<I, AliasError> {
        match self {
            Self::Ref(v) => v.into_id(),
            Self::Source(_) => Err(AliasError::Unresolved),
        }
    }
}
impl<I> CatalogSelector<I> {
    pub fn as_id(&self) -> Option<&I> {
        CatalogReference::as_id(self)
    }
    pub fn into_id(self) -> Result<I, AliasError> {
        CatalogReference::into_id(self)
    }
    pub fn is_alias(&self) -> bool {
        CatalogReference::is_alias(self)
    }
}
impl<I: TryFrom<String, Error = InvalidValue>> CatalogSelector<I> {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        if let Some(alias) = value.strip_prefix('@') {
            let (kind, id) = alias
                .split_once(':')
                .ok_or(InvalidValue("alias must be @namespace:value"))?;
            if !matches!(kind, "slug" | "name") {
                return SourceAlias::new(kind, id).map(Self::Source);
            }
        }
        CatalogRef::new(value).map(Self::Ref)
    }
}
impl<I> From<CatalogRef<I>> for CatalogSelector<I> {
    fn from(v: CatalogRef<I>) -> Self {
        Self::Ref(v)
    }
}
macro_rules! native_refs{($($id:ty),*)=>{$(impl From<$id> for CatalogSelector<$id>{fn from(v:$id)->Self{Self::Ref(CatalogRef::Id(v))}})*}}
native_refs!(CollectionId, ModelId, BackdropId, PatternId);
impl<I: TryFrom<String, Error = InvalidValue>> TryFrom<String> for CatalogSelector<I> {
    type Error = InvalidValue;
    fn try_from(v: String) -> Result<Self, InvalidValue> {
        Self::new(v)
    }
}
impl<I: TryFrom<String, Error = InvalidValue>> From<&'static str> for CatalogSelector<I> {
    fn from(v: &'static str) -> Self {
        Self::new(v).expect("valid static catalog selector")
    }
}
impl<I: fmt::Display> fmt::Display for CatalogSelector<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ref(v) => v.fmt(f),
            Self::Source(v) => write!(f, "@{}:{}", v.namespace, v.id),
        }
    }
}
impl<I: fmt::Display> CatalogSelector<I> {
    pub fn lookup_key(&self) -> String {
        match self {
            Self::Ref(v) => v.lookup_key(),
            Self::Source(v) => v.lookup_key(),
        }
    }
}
impl<I: fmt::Display> Serialize for CatalogSelector<I> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
impl<'de, I: TryFrom<String, Error = InvalidValue>> Deserialize<'de> for CatalogSelector<I> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
#[cfg(feature = "schema")]
impl<I> utoipa::PartialSchema for CatalogSelector<I> {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        <String as utoipa::PartialSchema>::schema()
    }
}
#[cfg(feature = "schema")]
impl<I> utoipa::ToSchema for CatalogSelector<I> {
    fn name() -> std::borrow::Cow<'static, str> {
        <CatalogRef<I> as utoipa::ToSchema>::name()
    }
}
pub fn filters(v: market_gifts::ResolvedGiftFilters) -> GiftFilters {
    GiftFilters {
        collection_id: v.collection_id.map(Into::into),
        currency_id: v.currency_id,
        venue_id: v.venue_id,
        models: v.models.into_iter().map(Into::into).collect(),
        backdrops: v.backdrops.into_iter().map(Into::into).collect(),
        patterns: v.patterns.into_iter().map(Into::into).collect(),
        number: v.number,
        price: v.price,
    }
}
pub fn query(v: market_gifts::ResolvedGiftOrderQuery) -> GiftOrderQuery {
    let f = filters(v.filters.clone());
    v.map_filters(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_source_selectors_roundtrip_only_at_the_adapter_boundary() {
        for value in ["@tg.slug:ResistanceDog", "@tg:42", "@partner:v2:id"] {
            let reference: CollectionRef =
                serde_json::from_value(serde_json::json!(value)).unwrap();
            assert!(matches!(reference, CatalogSelector::Source(_)));
            assert!(reference.clone().into_id().is_err());
            assert_eq!(reference.to_string(), value);
            assert_eq!(serde_json::to_value(reference).unwrap(), value);
        }
        let legacy = CollectionRef::new("@tg.slug:ResistanceDog").unwrap();
        let canonical = CollectionRef::new("@slug:resistancedog").unwrap();
        assert_eq!(legacy.lookup_key(), canonical.lookup_key());
        for value in [
            "@",
            "@tg:",
            "@:42",
            "@bad ns:42",
            "@slug:Has Space",
            "@name: x",
        ] {
            assert!(CollectionRef::new(value).is_err(), "{value}");
        }
        assert_eq!(
            CollectionRef::new("@tg:ABC").unwrap().lookup_key(),
            "@tg:ABC"
        );
    }
}
