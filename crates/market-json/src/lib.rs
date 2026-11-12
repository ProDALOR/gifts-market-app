//! Optional human JSON codec. Application projections are independent of hub implementation types.
mod codecs;
mod records;
mod strict;
mod values;
pub use codecs::{
    serialize_catalog, serialize_items, serialize_orders, serialize_records, u64_string,
};
pub use records::{
    catalog_value, event_value, item_value, order_value, parse_records, parse_sources,
    record_value, source_value,
};
pub use values::{
    RawAttributes, Scalar, attributes, decode_attributes, decode_scalar, parse_attributes,
    parse_raw_attributes,
};
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Validation(#[from] market_view::ValidationError),
}
pub type Result<T> = std::result::Result<T, Error>;

pub mod schema;
