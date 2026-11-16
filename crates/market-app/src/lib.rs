//! Application composition, configuration and process lifecycle.
pub mod api_config;
#[cfg(feature = "bot")]
pub mod bot_config;
pub mod cli;
pub mod market;
pub use market::MarketRuntime;
