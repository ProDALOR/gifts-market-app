//! Locally generated gRPC contract. No core implementation dependencies.
pub mod wire {
    tonic::include_proto!("market");
}
pub const PROTO: &str = include_str!("../../../market-hub.proto");
