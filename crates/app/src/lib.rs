//! Stateless gateway: translate public transports and forward market work to core.
pub mod cli;
pub mod error;
mod http;

pub mod query;

pub mod status;

pub use http::{ApiDoc, configure};
pub use query::ApiState;

pub mod client;

mod websocket;

pub use websocket::Origins;

pub use hub_protocol::wire;
