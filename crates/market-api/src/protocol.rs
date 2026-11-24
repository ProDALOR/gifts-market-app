//! Shared API protocol primitives. JSON stays independent of internal operation lifetimes.
use crate::error::{ApiError, Result};
use serde::{Deserialize, Serialize};
use utoipa::{
    PartialSchema, ToSchema,
    openapi::{
        RefOr,
        schema::{ObjectBuilder, Schema, Type},
    },
};
const MAX_ID_LENGTH: usize = 64;

/// Client-chosen correlation ID, unique among active operations on one connection.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct OperationId(String);
impl PartialSchema for OperationId {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .min_length(Some(1))
            .max_length(Some(MAX_ID_LENGTH))
            .pattern(Some("^[A-Za-z0-9_.-]+$"))
            .into()
    }
}
impl ToSchema for OperationId {}
impl OperationId {
    pub fn parse(value: String) -> Result<Self> {
        if value.is_empty()
            || value.len() > MAX_ID_LENGTH
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
        {
            return Err(ApiError::invalid("invalid operation ID"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for OperationId {
    type Error = ApiError;
    fn try_from(value: String) -> Result<Self> {
        Self::parse(value)
    }
}
impl From<&'static str> for OperationId {
    fn from(value: &'static str) -> Self {
        Self::parse(value.into()).expect("valid static operation ID")
    }
}

/// Command rejections may lack a valid operation identity; the wire uses an empty string.
pub(crate) fn serialize_correlation<S: serde::Serializer>(
    id: &Option<&OperationId>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(id.map(OperationId::as_str).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operation_id_schema_describes_the_same_plain_string_contract() {
        assert_eq!(
            serde_json::to_value(OperationId::schema()).unwrap(),
            serde_json::json!({"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9_.-]+$"}),
        );
    }
    #[test]
    fn operation_ids_validate_once_and_serialize_as_plain_strings() {
        for value in [
            String::new(),
            "x".repeat(65),
            "gift/id".into(),
            "with space".into(),
            "😀".into(),
        ] {
            assert!(OperationId::parse(value.clone()).is_err());
            assert!(serde_json::from_value::<OperationId>(serde_json::json!(value)).is_err());
        }
        for value in ["a".repeat(64), "op_1.next-2".into()] {
            let id = OperationId::parse(value.clone()).unwrap();
            assert_eq!(id.as_str(), value);
            assert_eq!(serde_json::to_value(&id).unwrap(), serde_json::json!(value));
            assert_eq!(
                serde_json::from_value::<OperationId>(serde_json::json!(value)).unwrap(),
                id
            );
        }
        for value in [
            serde_json::Value::Null,
            serde_json::json!(123),
            serde_json::json!({"id":"x"}),
        ] {
            assert!(serde_json::from_value::<OperationId>(value).is_err());
        }
    }
}
