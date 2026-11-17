use crate::InvalidValue;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{fmt, ops::Deref};

/// Nonnegative exact decimal, encoded as a string throughout the public contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", schema(value_type = String))]
pub struct Amount(String);
impl Amount {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        let decimal = Decimal::from_str_exact(&value)
            .map_err(|_| InvalidValue("invalid exact decimal amount"))?;
        decimal.try_into()
    }
    pub fn decimal(&self) -> Decimal {
        Decimal::from_str_exact(&self.0).expect("validated amount")
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<Decimal> for Amount {
    type Error = InvalidValue;
    fn try_from(decimal: Decimal) -> Result<Self, Self::Error> {
        if decimal < Decimal::ZERO {
            return Err(InvalidValue("amount must not be negative"));
        }
        Ok(Self(decimal.normalize().to_string()))
    }
}
impl TryFrom<String> for Amount {
    type Error = InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<Amount> for String {
    fn from(value: Amount) -> Self {
        value.0
    }
}
impl From<&'static str> for Amount {
    fn from(value: &'static str) -> Self {
        Self::new(value).expect("valid static amount")
    }
}
impl Deref for Amount {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
