use market_view as m;
use serde::{
    Deserialize,
    de::{Error, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};
/// A bounded duplicate-rejecting JSON tree; serde_json::Value alone overwrites duplicate keys.
pub(super) struct StrictValue(pub(super) Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: serde::Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        struct Strict;
        impl<'de> Visitor<'de> for Strict {
            type Value = StrictValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("bounded JSON without duplicate keys")
            }
            fn visit_bool<E: Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_i64<E: Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_u64<E: Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_f64<E: Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(StrictValue(
                    serde_json::Number::from_f64(v)
                        .ok_or_else(|| E::custom("invalid JSON number"))?
                        .into(),
                ))
            }
            fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_string<E: Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(StrictValue(v)) = input.next_element()? {
                    if values.len() == m::MAX_ATTRIBUTE_FILTER_VALUES {
                        return Err(A::Error::custom("too many JSON array entries"));
                    }
                    values.push(v);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some((key, StrictValue(v))) = input.next_entry::<String, StrictValue>()? {
                    if values.len() == 128 || values.insert(key, v).is_some() {
                        return Err(A::Error::custom(
                            "duplicate JSON key or too many object fields",
                        ));
                    }
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }
        input.deserialize_any(Strict)
    }
}
