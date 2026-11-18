//! Authenticated Telegram client, native operations and shared invocation scheduling.
mod config;
mod error;
mod inventory;
mod mtproto;
pub mod transport;
mod types;
pub use config::{BotConfig, BotCredentials, app_url};
pub use error::{BotError, ErrorDisposition};
pub use inventory::{
    GiftRelation, GiftSource, InventoryCompletion, InventoryError, InventoryPage, PartialReason,
    ProfileGift, SkipReason, VisibleInventory,
};
pub use mtproto::{Connection, TelegramGifts};
pub use transport::BotUser;
pub use types::{IncomingMessage, LaunchButton, MenuCommand, Reply};
#[cfg(test)]
mod tests;
