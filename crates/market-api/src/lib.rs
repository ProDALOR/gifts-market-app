//! HTTP/WebSocket adapters, gift-domain models and transport envelopes.
mod asyncapi;
mod config;
mod connections;
mod encoding;
pub mod error;
mod frontend;
mod market;
mod media;
#[cfg(feature = "miniapp")]
pub mod miniapp;
pub mod models;
mod origin;
mod routes;
mod schema;
mod security;
mod server;
mod websocket;
pub use config::{ApiConfig, ApiLimits, WebSocketLimits};
pub use market::MarketApi;
pub use routes::configure;
pub use schema::ApiDoc;
pub use security::security_headers;
pub use server::ApiServer;

/// Public schema used by the application CLI and frontend type generation.
pub fn schema_json() -> serde_json::Result<String> {
    use utoipa::OpenApi;
    ApiDoc::openapi().to_json()
}

/// AsyncAPI specification generated from the public WebSocket message types.
pub fn asyncapi_json() -> serde_json::Result<String> {
    serde_json::to_string(&asyncapi::document()?)
}
