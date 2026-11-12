//! API application: materialize a local read projection and serve public reads.
pub mod cli;
pub mod error;
mod http;
mod pagination;
pub mod query;
pub mod replication;
pub mod status;

pub use http::{ApiDoc, configure};
pub use query::ApiState;

pub mod grpc;
pub mod streams;
mod websocket;

pub use websocket::Origins;

pub mod wire {
    tonic::include_proto!("market_app.api");
}
