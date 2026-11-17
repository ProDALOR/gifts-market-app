//! Supervised Telegram transport, native gift inventory and authenticated media downloads.
mod config;
mod dispatcher;
mod error;
mod handler;
mod inventory;
mod mtproto;
mod transport;
mod worker;
pub use config::{BotConfig, BotCredentials};
pub use error::BotError;
pub use handler::{IncomingMessage, LaunchButton, MenuCommand, MessageHandler, Reply};
pub use inventory::{
    GiftRelation, GiftSource, InventoryCompletion, InventoryError, PartialReason, ProfileGift,
    SkipReason, VisibleInventory,
};
pub use transport::BotUser;
pub use worker::{BotInspection, TelegramWorker, inspect};
#[cfg(test)]
mod tests;
