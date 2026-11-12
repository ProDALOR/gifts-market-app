//! Serde field codecs keep Rust fields typed while choosing their public representation.
use market_view::{CatalogEntry, Item, Order, Record};
use serde::{Serializer, ser::Error, ser::SerializeSeq};
use std::borrow::Borrow;

/// Exact unsigned integers represented as decimal strings for JavaScript clients.
pub mod u64_string {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(value: &u64, output: S) -> Result<S::Ok, S::Error> {
        output.collect_str(value)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<u64, D::Error> {
        let value = String::deserialize(input)?;
        if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(D::Error::custom(
                "expected an unsigned decimal integer string",
            ));
        }
        value.parse().map_err(D::Error::custom)
    }
}

macro_rules! sequence_codec {
    ($name:ident, $model:ty, $encode:path) => {
        pub fn $name<S: Serializer, T: Borrow<$model>>(
            values: &[T],
            output: S,
        ) -> Result<S::Ok, S::Error> {
            let mut sequence = output.serialize_seq(Some(values.len()))?;
            for value in values {
                let value = $encode(value.borrow()).map_err(S::Error::custom)?;
                sequence.serialize_element(&value)?;
            }
            sequence.end()
        }
    };
}
sequence_codec!(serialize_items, Item, super::item_value);
sequence_codec!(serialize_records, Record, super::record_value);
sequence_codec!(serialize_orders, Order, super::order_value);
sequence_codec!(serialize_catalog, CatalogEntry, super::catalog_value);
