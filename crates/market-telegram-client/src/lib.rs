//! Authenticated Telegram client, native operations and shared invocation scheduling.
mod config;
mod error;
pub mod interactions;
mod mtproto;
pub mod transport;
mod types;
pub use config::{BotConfig, BotCredentials, app_url};
pub use error::{BotError, ErrorDisposition};
pub use mtproto::{
    AccountExports, AccountGiftResolver, AccountWorker, Connection, InventoryClient, LoginStep,
    UserLogin,
};
pub use transport::BotUser;
pub use types::{IncomingMessage, LaunchButton, MenuCommand, Reply, ReplyButton, ReplyKind};
#[cfg(test)]
mod tests;
