//! gRPC decoding into local query projections; no server implementation is imported.
pub use hub_protocol::{
    MAX_BATCH_RECORDS, MAX_MESSAGE_BYTES, SYNC_FRAME_BYTES, SYNC_FRAME_RECORDS, wire,
};
pub mod catalog;
mod mapping;
