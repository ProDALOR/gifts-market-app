//! Private grammers adapter: TL calls, payloads, session storage and gift mapping.
mod accounts;
mod client;
pub use accounts::{AccountGiftResolver, AccountWorker};
mod connection;
mod gifts;
mod invocations;
mod media;
mod requests;
mod session_store;
pub use connection::Connection;
pub use gifts::InventoryClient;
#[cfg(test)]
mod tests;
