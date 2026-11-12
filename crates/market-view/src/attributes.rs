use crate::*;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};
pub const MAX_ATTRIBUTES: usize = 64;
pub const MAX_ATTRIBUTE_FILTER_VALUES: usize = 128;
pub const MAX_ATTRIBUTE_TEXT_BYTES: usize = 2048;
pub type Attributes = BTreeMap<AttributeId, AttributeValue>;
pub type AttributeFilter = BTreeMap<AttributeId, AttributePredicate>;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AttributeValue {
    Enum(AttributeValueId),
    Integer(#[serde(with = "integer_string")] i64),
    Decimal(#[serde(with = "crate::primitives::decimal")] Decimal),
    Boolean(bool),
    Text(String),
    Timestamp(MarketTimestamp),
}
pub mod integer_string {
    pub fn serialize<S: serde::Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        use serde::Deserialize;
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
impl AttributeValue {
    pub fn decimal(s: &str) -> Result<Self, ValidationError> {
        Ok(Self::Decimal(Decimal::from_str_exact(s).map_err(|_| {
            ValidationError("invalid decimal attribute")
        })?))
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        if matches!(self,Self::Text(s) if s.len()>MAX_ATTRIBUTE_TEXT_BYTES) {
            Err(ValidationError("attribute text too long"))
        } else {
            Ok(())
        }
    }
    pub fn range_cmp(&self, b: &Self) -> Option<Ordering> {
        match (self, b) {
            (Self::Integer(a), Self::Integer(b)) => Some(a.cmp(b)),
            (Self::Decimal(a), Self::Decimal(b)) => Some(a.cmp(b)),
            (Self::Timestamp(a), Self::Timestamp(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }
    pub fn value_type(&self) -> AttributeType {
        match self {
            Self::Enum(_) => AttributeType::Enum,
            Self::Integer(_) => AttributeType::Integer,
            Self::Decimal(_) => AttributeType::Decimal,
            Self::Boolean(_) => AttributeType::Boolean,
            Self::Text(_) => AttributeType::Text,
            Self::Timestamp(_) => AttributeType::Timestamp,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum AttributePredicate {
    Eq {
        value: AttributeValue,
    },
    OneOf {
        values: BTreeSet<AttributeValue>,
    },
    Range {
        #[serde(default)]
        min: Option<AttributeValue>,
        #[serde(default)]
        max: Option<AttributeValue>,
    },
}
impl AttributePredicate {
    pub fn values(&self) -> Vec<&AttributeValue> {
        match self {
            Self::Eq { value } => vec![value],
            Self::OneOf { values } => values.iter().collect(),
            Self::Range { min, max } => min.iter().chain(max.iter()).collect(),
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        let values = self.values();
        for v in &values {
            v.validate()?
        }
        match self {
            Self::OneOf { values }
                if values.is_empty()
                    || values.len() > 128
                    || values.iter().any(|v| {
                        !matches!(
                            v,
                            AttributeValue::Enum(_)
                                | AttributeValue::Integer(_)
                                | AttributeValue::Decimal(_)
                        ) || v.value_type() != values.first().unwrap().value_type()
                    }) =>
            {
                Err(ValidationError("invalid one_of filter"))
            }
            Self::Range { min, max } if min.is_none() && max.is_none() => {
                Err(ValidationError("range requires an endpoint"))
            }
            Self::Range { min, max } => {
                let first = min.as_ref().or(max.as_ref()).unwrap();
                if first.range_cmp(first).is_none()
                    || min
                        .as_ref()
                        .zip(max.as_ref())
                        .is_some_and(|(a, b)| a.range_cmp(b).is_none_or(|c| c == Ordering::Greater))
                {
                    Err(ValidationError("invalid range"))
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }
}

pub fn deserialize_attribute_map<'de, D, V>(d: D) -> Result<BTreeMap<AttributeId, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct MapVisitor<V>(std::marker::PhantomData<V>);
    impl<'de, V: Deserialize<'de>> serde::de::Visitor<'de> for MapVisitor<V> {
        type Value = BTreeMap<AttributeId, V>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("distinct attribute IDs")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut a: A,
        ) -> Result<Self::Value, A::Error> {
            let mut r = BTreeMap::new();
            while let Some((k, v)) = a.next_entry()? {
                if r.len() == 64 || r.insert(k, v).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate attribute or too many values",
                    ));
                }
            }
            Ok(r)
        }
    }
    d.deserialize_map(MapVisitor(std::marker::PhantomData))
}
impl<const N: usize> From<[AttributeValue; N]> for AttributePredicate {
    fn from(values: [AttributeValue; N]) -> Self {
        Self::OneOf {
            values: values.into_iter().collect(),
        }
    }
}
