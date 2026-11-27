//! Private grammers adapter: TL calls, payloads, session storage and gift mapping.
mod accounts;
mod user_login;
pub use user_login::{AccountExports, LoginStep, UserLogin};
mod client;
pub use accounts::{AccountGiftResolver, AccountWorker};
mod connection;
mod emojis;
mod gifts;
mod interactions;
mod invocations;
mod media;
mod photos;
mod requests;
mod session_store;
pub use connection::Connection;
pub use gifts::InventoryClient;
#[cfg(test)]
mod tests;
