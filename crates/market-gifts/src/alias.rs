//! Application catalog references. Source-specific identities belong to adapters.
use crate::{BackdropId, CollectionId, InvalidValue, ModelId, PatternId};
use serde::{Deserialize, Serialize};
use std::fmt;
macro_rules! text_reference {
    ($name:ident, $prefix:literal, $slug:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        #[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
        #[cfg_attr(feature="schema", schema(value_type=String))]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
                let value = value.into();
                if value.is_empty()
                    || value.len() + $prefix.len() > 256
                    || value.trim() != value
                    || value.chars().any(char::is_control)
                    || ($slug
                        && !value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
                {
                    return Err(InvalidValue("invalid catalog lookup value"));
                }
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = InvalidValue;
            fn try_from(value: String) -> Result<Self, InvalidValue> {
                Self::new(value)
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}
text_reference!(CatalogSlug, "@slug:", true);
text_reference!(CatalogName, "@name:", false);

/// Public catalog aliases describe lookup semantics, never their upstream source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(
    tag = "namespace",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftAlias {
    Slug(CatalogSlug),
    Name(CatalogName),
}
impl GiftAlias {
    pub fn new(kind: &str, value: impl Into<String>) -> Result<Self, InvalidValue> {
        match kind {
            "slug" => CatalogSlug::new(value).map(Self::Slug),
            "name" => CatalogName::new(value).map(Self::Name),
            _ => Err(InvalidValue("catalog alias must use slug or name")),
        }
    }
    pub fn id(&self) -> &str {
        match self {
            Self::Slug(v) => v.as_str(),
            Self::Name(v) => v.as_str(),
        }
    }
    pub fn selector(&self) -> String {
        match self {
            Self::Slug(v) => format!("@slug:{v}"),
            Self::Name(v) => format!("@name:{v}"),
        }
    }
    pub fn lookup_key(&self) -> String {
        self.selector().to_ascii_lowercase()
    }
    pub fn matches(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
            && self.id().eq_ignore_ascii_case(other.id())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CatalogRef<I> {
    Id(I),
    Slug(CatalogSlug),
    Name(CatalogName),
}
pub type CollectionRef = CatalogRef<CollectionId>;
pub type ModelRef = CatalogRef<ModelId>;
pub type BackdropRef = CatalogRef<BackdropId>;
pub type PatternRef = CatalogRef<PatternId>;
/// Input adapters can retain legacy source IDs without admitting them as native identities.
pub trait CatalogReference: Sized {
    type Id;
    fn as_id(&self) -> Option<&Self::Id>;
    fn into_id(self) -> Result<Self::Id, AliasError>;
    fn is_alias(&self) -> bool {
        self.as_id().is_none()
    }
}
impl<I> CatalogReference for CatalogRef<I> {
    type Id = I;
    fn as_id(&self) -> Option<&I> {
        if let Self::Id(id) = self {
            Some(id)
        } else {
            None
        }
    }
    fn into_id(self) -> Result<I, AliasError> {
        if let Self::Id(id) = self {
            Ok(id)
        } else {
            Err(AliasError::Unresolved)
        }
    }
}
impl<I> CatalogRef<I> {
    pub fn as_id(&self) -> Option<&I> {
        CatalogReference::as_id(self)
    }
    pub fn is_alias(&self) -> bool {
        CatalogReference::is_alias(self)
    }
    pub fn into_id(self) -> Result<I, AliasError> {
        CatalogReference::into_id(self)
    }
}
impl<I: TryFrom<String, Error = InvalidValue>> CatalogRef<I> {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        if let Some(alias) = value.strip_prefix('@') {
            let (kind, value) = alias
                .split_once(':')
                .ok_or(InvalidValue("alias must be @slug:value or @name:value"))?;
            match kind {
                "slug" => CatalogSlug::new(value).map(Self::Slug),
                "name" => CatalogName::new(value).map(Self::Name),
                _ => Err(InvalidValue("catalog reference must use slug or name")),
            }
        } else {
            I::try_from(value).map(Self::Id)
        }
    }
}
impl<I: TryFrom<String, Error = InvalidValue>> TryFrom<String> for CatalogRef<I> {
    type Error = InvalidValue;
    fn try_from(v: String) -> Result<Self, InvalidValue> {
        Self::new(v)
    }
}
impl<I: TryFrom<String, Error = InvalidValue>> From<&'static str> for CatalogRef<I> {
    fn from(v: &'static str) -> Self {
        Self::new(v).expect("valid static catalog reference")
    }
}
macro_rules! native_refs {($($id:ty),*)=>{$(impl From<$id> for CatalogRef<$id>{fn from(v:$id)->Self{Self::Id(v)}})*}}
native_refs!(CollectionId, ModelId, BackdropId, PatternId);
impl<I> From<GiftAlias> for CatalogRef<I> {
    fn from(v: GiftAlias) -> Self {
        match v {
            GiftAlias::Slug(v) => Self::Slug(v),
            GiftAlias::Name(v) => Self::Name(v),
        }
    }
}
impl<I: fmt::Display> fmt::Display for CatalogRef<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(v) => v.fmt(f),
            Self::Slug(v) => write!(f, "@slug:{v}"),
            Self::Name(v) => write!(f, "@name:{v}"),
        }
    }
}
impl<I: fmt::Display> CatalogRef<I> {
    pub fn lookup_key(&self) -> String {
        match self {
            Self::Id(v) => v.to_string(),
            _ => self.to_string().to_ascii_lowercase(),
        }
    }
}
impl<I: fmt::Display> Serialize for CatalogRef<I> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
impl<'de, I: TryFrom<String, Error = InvalidValue>> Deserialize<'de> for CatalogRef<I> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
#[cfg(feature = "schema")]
impl<I> utoipa::PartialSchema for CatalogRef<I> {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        <String as utoipa::PartialSchema>::schema()
    }
}
#[cfg(feature = "schema")]
impl<I> utoipa::ToSchema for CatalogRef<I> {
    fn name() -> std::borrow::Cow<'static, str> {
        format!(
            "{}Ref",
            std::any::type_name::<I>()
                .rsplit("::")
                .next()
                .unwrap_or("Catalog")
                .trim_end_matches("Id")
        )
        .into()
    }
}

/// Gift attribute identity, independent of the hub's attribute namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftTraitKind {
    Model,
    Backdrop,
    Pattern,
}

/// Resolution failures are classified without inspecting human-readable messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasError {
    Malformed(InvalidValue),
    UnknownCollection,
    AmbiguousCollection,
    UnknownTrait(GiftTraitKind),
    AmbiguousTrait(GiftTraitKind),
    ConflictingBackdrop,
    ModelRequiresCollection,
    BudgetExceeded(&'static str),
    Cancelled,
    InvalidCatalog(&'static str),
    Unresolved,
}
impl From<InvalidValue> for AliasError {
    fn from(e: InvalidValue) -> Self {
        Self::Malformed(e)
    }
}
impl fmt::Display for AliasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed(e) => return e.fmt(f),
            Self::UnknownCollection => "unknown collection or alias",
            Self::AmbiguousCollection => "ambiguous collection alias",
            Self::UnknownTrait(_) => "unknown trait or alias in selected collections",
            Self::AmbiguousTrait(_) => "ambiguous trait alias within collection",
            Self::ConflictingBackdrop => "ambiguous backdrop alias across collections",
            Self::ModelRequiresCollection => "model filters require a collection",
            Self::BudgetExceeded(reason) | Self::InvalidCatalog(reason) => reason,
            Self::Cancelled => "catalog alias projection cancelled",
            Self::Unresolved => "catalog alias must be resolved before reading native data",
        })
    }
}
impl std::error::Error for AliasError {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_have_only_native_slug_and_name_variants() {
        for (value, expected) in [
            ("native.id", 0),
            ("@slug:ResistanceDog", 1),
            ("@name:Dark Knight", 2),
        ] {
            let reference: CollectionRef =
                serde_json::from_value(serde_json::json!(value)).unwrap();
            assert_eq!(serde_json::to_value(&reference).unwrap(), value);
            assert_eq!(
                match &reference {
                    CatalogRef::Id(_) => 0,
                    CatalogRef::Slug(_) => 1,
                    CatalogRef::Name(_) => 2,
                },
                expected
            );
            let map = std::collections::BTreeMap::from([(reference, "v")]);
            assert_eq!(
                serde_json::to_value(map).unwrap(),
                serde_json::json!({value:"v"})
            );
        }
        for external in ["@tg:123", "@tg.slug:Gift", "@partner:id"] {
            assert!(CollectionRef::new(external).is_err());
        }
    }
    #[test]
    fn public_aliases_have_two_kinds_and_keep_the_object_contract() {
        for (kind, value) in [("slug", "ResistanceDog"), ("name", "Dark Knight")] {
            let alias = GiftAlias::new(kind, value).unwrap();
            let json = serde_json::json!({"namespace":kind,"id":value});
            assert_eq!(serde_json::to_value(&alias).unwrap(), json);
            assert_eq!(serde_json::from_value::<GiftAlias>(json).unwrap(), alias);
            assert_eq!(
                alias.lookup_key(),
                format!("@{kind}:{}", value.to_ascii_lowercase())
            );
        }
        assert!(GiftAlias::new("tg", "123").is_err());
        assert!(
            serde_json::from_value::<GiftAlias>(
                serde_json::json!({"namespace":"slug","id":"Gift","extra":true})
            )
            .is_err()
        );
    }
    #[test]
    fn malformed_names_and_slugs_fail_before_resolution() {
        for value in [
            "@",
            "@slug:",
            "@name:",
            "@slug:Has Space",
            "@name: x",
            "@name:x\n",
        ] {
            assert!(CollectionRef::new(value).is_err(), "{value}");
        }
        let reference = CollectionRef::new("@slug:Gift").unwrap();
        assert_eq!(reference.into_id(), Err(AliasError::Unresolved));
        assert!(CollectionId::new("@slug:Gift").is_err());
        let input: crate::GiftFilters = crate::GiftFilters::new("gift".into(), "GRAM".into());
        assert_eq!(
            serde_json::to_value(&input).unwrap(),
            serde_json::to_value(input.into_resolved().unwrap()).unwrap()
        );
    }
}
