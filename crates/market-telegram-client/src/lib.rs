//! Authenticated Telegram client, native operations and shared invocation scheduling.
mod config;
mod error;
mod mtproto;
pub mod transport;
mod types;
pub use config::{BotConfig, BotCredentials, app_url};
pub use error::{BotError, ErrorDisposition};
pub use mtproto::{Connection, InventoryClient};
pub use transport::BotUser;
pub use types::{IncomingMessage, LaunchButton, MenuCommand, Reply};
#[cfg(test)]
mod tests;
