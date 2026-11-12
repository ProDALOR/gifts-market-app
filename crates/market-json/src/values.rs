use super::Result;
use market_view as m;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
/// Numeric values are strings so browsers never round an integer or decimal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    String(String),
    Boolean(bool),
}
impl From<&m::AttributeValue> for Scalar {
    fn from(v: &m::AttributeValue) -> Self {
        match v {
            m::AttributeValue::Enum(v) => Self::String(v.to_string()),
            m::AttributeValue::Integer(v) => Self::String(v.to_string()),
            m::AttributeValue::Decimal(v) => Self::String(v.to_string()),
            m::AttributeValue::Text(v) => Self::String(v.clone()),
            m::AttributeValue::Boolean(v) => Self::Boolean(*v),
            m::AttributeValue::Timestamp(v) => Self::String(v.to_string()),
        }
    }
}
pub type RawAttributes = BTreeMap<m::AttributeId, Scalar>;
pub fn attributes(values: &m::Attributes) -> RawAttributes {
    values
        .iter()
        .map(|(id, value)| (id.clone(), value.into()))
        .collect()
}
fn value(
    catalog: &m::Catalog,
    kind: &m::ItemTypeId,
    id: &m::AttributeId,
    scalar: Scalar,
) -> Result<m::AttributeValue> {
    if !catalog
        .item_type(kind)
        .ok_or(m::ValidationError("unknown item type ID"))?
        .attributes
        .contains_key(id)
    {
        return Err(m::ValidationError("attribute is not permitted for this item type").into());
    }
    let schema = catalog
        .attribute(id)
        .ok_or(m::ValidationError("unknown attribute ID"))?;
    Ok(match (schema.value_type, scalar) {
        (m::AttributeType::Enum, Scalar::String(v)) => m::AttributeValue::Enum(v.parse()?),
        (m::AttributeType::Text, Scalar::String(v)) => m::AttributeValue::Text(v),
        (m::AttributeType::Integer, Scalar::String(v)) => m::AttributeValue::Integer(
            v.parse()
                .map_err(|_| m::ValidationError("invalid integer attribute"))?,
        ),
        (m::AttributeType::Decimal, Scalar::String(v)) => m::AttributeValue::decimal(&v)?,
        (m::AttributeType::Timestamp, Scalar::String(v)) => {
            m::AttributeValue::Timestamp(v.parse()?)
        }
        (m::AttributeType::Boolean, Scalar::Boolean(v)) => m::AttributeValue::Boolean(v),
        _ => {
            return Err(
                m::ValidationError("attribute JSON type differs from catalog schema").into(),
            );
        }
    })
}
pub fn decode_attributes(
    catalog: &m::Catalog,
    kind: &m::ItemTypeId,
    raw: RawAttributes,
) -> Result<m::Attributes> {
    if raw.len() > m::MAX_ATTRIBUTES {
        return Err(m::ValidationError("too many attributes").into());
    }
    let values = raw
        .into_iter()
        .map(|(id, v)| Ok((id.clone(), value(catalog, kind, &id, v)?)))
        .collect::<Result<_>>()?;
    catalog.validate_attributes(kind, &values)?;
    Ok(values)
}
#[derive(Deserialize)]
struct AttributeMap(
    #[serde(deserialize_with = "m::attributes::deserialize_attribute_map")] RawAttributes,
);
pub fn parse_attributes(
    bytes: &[u8],
    catalog: &m::Catalog,
    kind: &m::ItemTypeId,
) -> Result<m::Attributes> {
    decode_attributes(
        catalog,
        kind,
        serde_json::from_slice::<AttributeMap>(bytes)?.0,
    )
}
