use serde::{Deserialize, Serialize};
use std::{fmt, ops::Deref};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidValue(pub &'static str);
impl fmt::Display for InvalidValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for InvalidValue {}

macro_rules! identifiers {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        #[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
        #[cfg_attr(feature = "schema", schema(value_type = String))]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
                let value = value.into();
                if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) || value.trim() != value {
                    return Err(InvalidValue("identifier must contain 1..256 bytes without surrounding whitespace or control characters"));
                }
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str { &self.0 }
        }
        impl TryFrom<String> for $name {
            type Error = InvalidValue;
            fn try_from(value: String) -> Result<Self, Self::Error> { Self::new(value) }
        }
        impl From<$name> for String { fn from(value: $name) -> Self { value.0 } }
        impl From<&'static str> for $name {
            fn from(value: &'static str) -> Self { Self::new(value).expect("valid static identifier") }
        }
        impl Deref for $name { type Target = str; fn deref(&self) -> &str { &self.0 } }
        impl std::borrow::Borrow<str> for $name { fn borrow(&self) -> &str { &self.0 } }
        impl AsRef<str> for $name { fn as_ref(&self) -> &str { &self.0 } }
        impl fmt::Display for $name { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) } }
    )+};
}
identifiers!(
    CollectionId,
    GiftId,
    ModelId,
    BackdropId,
    PatternId,
    VenueId,
    CurrencyId,
    OrderId
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", schema(value_type = String))]
pub struct RgbColor(String);
impl TryFrom<String> for RgbColor {
    type Error = InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(InvalidValue("color must be #RRGGBB"))
        }
    }
}
impl From<RgbColor> for String {
    fn from(value: RgbColor) -> Self {
        value.0
    }
}
impl fmt::Display for RgbColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Telegram long identifiers stay strings in JSON, preserving all 64 bits in JavaScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", schema(value_type = String))]
pub struct TelegramDocumentId(i64);
impl TelegramDocumentId {
    pub fn new(value: i64) -> Result<Self, InvalidValue> {
        if value <= 0 {
            Err(InvalidValue("Telegram document ID must be positive"))
        } else {
            Ok(Self(value))
        }
    }
    pub fn get(self) -> i64 {
        self.0
    }
}
impl TryFrom<String> for TelegramDocumentId {
    type Error = InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(
            value
                .parse()
                .map_err(|_| InvalidValue("invalid Telegram document ID"))?,
        )
    }
}
impl From<TelegramDocumentId> for String {
    fn from(value: TelegramDocumentId) -> Self {
        value.0.to_string()
    }
}

/// Gift collection IDs are distinct from sticker/emoji document IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", schema(value_type = String))]
pub struct TelegramGiftId(i64);
impl TelegramGiftId {
    pub fn new(value: i64) -> Result<Self, InvalidValue> {
        if value <= 0 {
            Err(InvalidValue("Telegram gift ID must be positive"))
        } else {
            Ok(Self(value))
        }
    }
    pub fn get(self) -> i64 {
        self.0
    }
}
impl TryFrom<String> for TelegramGiftId {
    type Error = InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(
            value
                .parse()
                .map_err(|_| InvalidValue("invalid Telegram gift ID"))?,
        )
    }
}
impl From<TelegramGiftId> for String {
    fn from(value: TelegramGiftId) -> Self {
        value.0.to_string()
    }
}
