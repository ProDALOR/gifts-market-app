//! Locally generated gRPC contract. No core implementation dependencies.
pub mod wire {
    tonic::include_proto!("market");
}
pub const PROTO: &str = include_str!("../../../market-hub.proto");
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_BATCH_RECORDS: usize = 500;
pub const SYNC_FRAME_RECORDS: usize = 500;
pub const SYNC_FRAME_BYTES: usize = MAX_MESSAGE_BYTES / 2;
