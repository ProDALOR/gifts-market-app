//! Shared portfolio orchestration. No Telegram client, HTTP or bot integration.
mod actor;
mod config;
mod inventory;
mod provider;
mod read;
pub use read::*;
mod service;
mod types;
pub use config::PortfolioConfig;
pub use provider::{InventoryProvider, ValuationProvider};
pub use service::{PortfolioHandle, PortfolioService, PortfolioTask, PortfolioWatch};
pub use types::*;
#[cfg(test)]
mod tests;
