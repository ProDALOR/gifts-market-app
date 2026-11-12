use crate::ValidationError;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
pub fn validate_id(s: &str) -> Result<(), ValidationError> {
    if s.is_empty() || s.len() > 256 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Err(ValidationError("invalid identifier"))
    } else {
        Ok(())
    }
}
macro_rules! ids {($($id:ident),*)=>{$(
#[derive(Debug,Clone,PartialEq,Eq,PartialOrd,Ord,Hash,Serialize,Deserialize)]
#[serde(try_from="String",into="String")]
pub struct $id(String);
impl $id {pub fn new(s:impl Into<String>)->Result<Self,ValidationError>{let s=s.into();validate_id(&s)?;Ok(Self(s))}pub fn as_str(&self)->&str{&self.0}pub fn into_string(self)->String{self.0}}
impl TryFrom<String> for $id {type Error=ValidationError;fn try_from(s:String)->Result<Self,Self::Error>{Self::new(s)}}
impl From<$id> for String {fn from(s:$id)->String{s.0}}
impl FromStr for $id {type Err=ValidationError;fn from_str(s:&str)->Result<Self,Self::Err>{Self::new(s)}}
impl fmt::Display for $id {fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result{f.write_str(&self.0)}}
impl std::borrow::Borrow<str> for $id {fn borrow(&self)->&str{&self.0}}
impl AsRef<str> for $id {fn as_ref(&self)->&str{&self.0}}
)*}}
ids!(
    FeedId,
    VenueId,
    CurrencyId,
    ItemTypeId,
    ItemId,
    OrderId,
    TradeId,
    OrderEventId,
    AttributeId,
    AttributeValueId
);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MarketTimestamp(jiff::Timestamp);
impl MarketTimestamp {
    pub fn from_millis(ms: i64) -> Result<Self, ValidationError> {
        if ms <= 0 {
            return Err(ValidationError("invalid observation timestamp"));
        }
        jiff::Timestamp::from_millisecond(ms)
            .map(Self)
            .map_err(|_| ValidationError("timestamp out of range"))
    }
    pub fn now() -> Self {
        Self::from_millis(jiff::Timestamp::now().as_millisecond()).expect("system clock")
    }
    pub fn as_millis(self) -> i64 {
        self.0.as_millisecond()
    }
    pub fn as_jiff(self) -> jiff::Timestamp {
        self.0
    }
    pub fn duration_since(self, other: Self) -> jiff::SignedDuration {
        self.0.duration_since(other.0)
    }
    pub fn checked_sub(self, d: jiff::SignedDuration) -> Result<Self, ValidationError> {
        self.0
            .checked_sub(d)
            .map_err(|_| ValidationError("timestamp out of range"))?
            .try_into()
    }
}
impl TryFrom<jiff::Timestamp> for MarketTimestamp {
    type Error = ValidationError;
    fn try_from(t: jiff::Timestamp) -> Result<Self, Self::Error> {
        if t.subsec_nanosecond() % 1_000_000 != 0 {
            return Err(ValidationError("timestamp must use whole milliseconds"));
        }
        Self::from_millis(t.as_millisecond())
    }
}
impl TryFrom<i64> for MarketTimestamp {
    type Error = ValidationError;
    fn try_from(t: i64) -> Result<Self, Self::Error> {
        Self::from_millis(t)
    }
}
impl FromStr for MarketTimestamp {
    type Err = ValidationError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<jiff::Timestamp>()
            .map_err(|_| ValidationError("invalid timestamp"))?
            .try_into()
    }
}
impl TryFrom<String> for MarketTimestamp {
    type Error = ValidationError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}
impl From<MarketTimestamp> for String {
    fn from(t: MarketTimestamp) -> String {
        t.to_string()
    }
}
impl From<MarketTimestamp> for i64 {
    fn from(t: MarketTimestamp) -> i64 {
        t.as_millis()
    }
}
impl fmt::Display for MarketTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Quantity(Decimal);
impl Quantity {
    pub const ONE: Self = Self(Decimal::ONE);
    pub fn one() -> Self {
        Self::ONE
    }
    pub fn new(d: Decimal) -> Result<Self, ValidationError> {
        if d <= Decimal::ZERO {
            Err(ValidationError("quantity must be positive"))
        } else {
            Ok(Self(d.normalize()))
        }
    }
    pub fn as_decimal(self) -> Decimal {
        self.0
    }
    pub fn is_multiple_of(self, step: Self) -> bool {
        self.0.checked_rem(step.0).is_some_and(|d| d.is_zero())
    }
}
impl FromStr for Quantity {
    type Err = ValidationError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(Decimal::from_str_exact(s).map_err(|_| ValidationError("invalid quantity"))?)
    }
}
impl TryFrom<String> for Quantity {
    type Error = ValidationError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}
impl From<Quantity> for String {
    fn from(q: Quantity) -> String {
        q.to_string()
    }
}
impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub mod decimal {
    use rust_decimal::Decimal;
    pub fn serialize<S: serde::Serializer>(v: &Decimal, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Decimal, D::Error> {
        use serde::Deserialize;
        Decimal::from_str_exact(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
