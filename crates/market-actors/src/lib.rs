//! Read-only order-book, catalog and market-subscription actors.
mod catalog;
pub mod error;
mod handle;
mod orderbooks;
mod protocol;
mod query;
mod runtime;
mod service;
mod subscriptions;
#[cfg(test)]
mod tests;
pub use catalog::CatalogSnapshot;
pub use handle::MarketDataHandle;
pub use market_replica::{READ_LIMITS, ReadLimits};
pub use service::{MarketDataService, MarketDataTask};
pub use subscriptions::{DataSubscription, DataUpdate};
