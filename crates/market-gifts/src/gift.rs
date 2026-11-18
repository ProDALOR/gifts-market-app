//! Collectible identity, positive edition numbers and appearance facts.
use crate::{BackdropId, CollectionId, GiftId, InvalidValue, ModelId, PatternId};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    hash::{Hash, Hasher},
};

/// A collectible's positive edition number, encoded as a JSON integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct GiftNumber(std::num::NonZeroU32);
#[cfg(feature = "schema")]
impl utoipa::PartialSchema for GiftNumber {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Integer)
            .minimum(Some(1u32))
            .maximum(Some(u32::MAX))
            .into()
    }
}
#[cfg(feature = "schema")]
impl utoipa::ToSchema for GiftNumber {}
impl GiftNumber {
    pub fn new(value: u32) -> Result<Self, InvalidValue> {
        std::num::NonZeroU32::new(value)
            .map(Self)
            .ok_or(InvalidValue("gift number must be positive"))
    }
    pub fn get(self) -> u32 {
        self.0.get()
    }
}
impl TryFrom<u32> for GiftNumber {
    type Error = InvalidValue;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<GiftNumber> for u32 {
    fn from(value: GiftNumber) -> Self {
        value.get()
    }
}
impl std::fmt::Display for GiftNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Canonical collection prefix supplied by catalog metadata. Display spelling is preserved.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CollectionSlug {
    spelling: String,
    key: String,
}
impl CollectionSlug {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let spelling = value.into();
        if spelling.is_empty()
            || spelling.len() > 128
            || !spelling.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(InvalidValue(
                "collection slug must contain 1..128 ASCII letters or digits",
            ));
        }
        let key = spelling.to_ascii_lowercase();
        Ok(Self { spelling, key })
    }
    pub fn as_str(&self) -> &str {
        &self.spelling
    }
    pub fn lookup_key(&self) -> &str {
        &self.key
    }
}
impl PartialEq for CollectionSlug {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for CollectionSlug {}
impl PartialOrd for CollectionSlug {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for CollectionSlug {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}
impl Hash for CollectionSlug {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}
impl TryFrom<String> for CollectionSlug {
    type Error = InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<CollectionSlug> for String {
    fn from(value: CollectionSlug) -> Self {
        value.spelling
    }
}
impl std::fmt::Display for CollectionSlug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.spelling.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GiftSlug {
    collection: CollectionSlug,
    number: GiftNumber,
}
impl GiftSlug {
    pub fn new(input: impl Into<String>) -> Result<Self, InvalidValue> {
        let input = input.into();
        let input = input.trim();
        let invalid =
            || InvalidValue("expected a gift slug such as PlushPepe-123 or a Telegram /nft/ link");
        if input.len() > 512 {
            return Err(invalid());
        }
        let value = if input.contains('/') || input.contains(':') {
            let text = if input.starts_with("t.me/") || input.starts_with("telegram.me/") {
                format!("https://{input}")
            } else {
                input.to_owned()
            };
            let url = url::Url::parse(&text).map_err(|_| invalid())?;
            if !matches!(url.scheme(), "https" | "http")
                || !matches!(url.host_str(), Some("t.me" | "telegram.me"))
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
            {
                return Err(invalid());
            }
            let path = url.path().strip_prefix("/nft/").ok_or_else(invalid)?;
            if path.contains('/') {
                return Err(invalid());
            }
            path.to_owned()
        } else {
            input.to_owned()
        };
        let (collection, number) = value.rsplit_once('-').ok_or_else(invalid)?;
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        let collection = CollectionSlug::new(collection).map_err(|_| invalid())?;
        let number =
            GiftNumber::new(number.parse().map_err(|_| invalid())?).map_err(|_| invalid())?;
        Ok(Self { collection, number })
    }
    pub fn collection(&self) -> &CollectionSlug {
        &self.collection
    }
    pub fn number(&self) -> GiftNumber {
        self.number
    }
    pub fn lookup_key(&self) -> String {
        format!("{}-{}", self.collection.lookup_key(), self.number.get())
    }
}
impl TryFrom<String> for GiftSlug {
    type Error = InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<GiftSlug> for String {
    fn from(value: GiftSlug) -> Self {
        value.to_string()
    }
}
impl std::fmt::Display for GiftSlug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.collection, self.number.get())
    }
}

// Both structured identities use their validated string representation on the wire.
#[cfg(feature = "schema")]
impl utoipa::PartialSchema for CollectionSlug {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        <String as utoipa::PartialSchema>::schema()
    }
}
#[cfg(feature = "schema")]
impl utoipa::ToSchema for CollectionSlug {}
#[cfg(feature = "schema")]
impl utoipa::PartialSchema for GiftSlug {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        <String as utoipa::PartialSchema>::schema()
    }
}
#[cfg(feature = "schema")]
impl utoipa::ToSchema for GiftSlug {}

/// Resolve synchronized gift facts by a public slug or stable catalog identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum GiftLookup {
    Slug(GiftSlug),
    Catalog {
        collection_id: CollectionId,
        number: GiftNumber,
    },
}
impl From<GiftSlug> for GiftLookup {
    fn from(slug: GiftSlug) -> Self {
        Self::Slug(slug)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GiftTraits {
    pub model: Option<ModelId>,
    pub backdrop: Option<BackdropId>,
    pub pattern: Option<PatternId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CollectibleGift {
    pub id: GiftId,
    pub collection_id: CollectionId,
    pub number: Option<GiftNumber>,
    pub traits: GiftTraits,
}
