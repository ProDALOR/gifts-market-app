//! Supervised Telegram update delivery and application command dispatch.
mod dispatcher;
mod handler;
mod worker;
pub use handler::MessageHandler;
pub use market_telegram_client::{
    BotConfig, BotCredentials, BotError, BotUser, GiftRelation, GiftSource, IncomingMessage,
    InventoryCompletion, InventoryError, LaunchButton, MenuCommand, PartialReason, ProfileGift,
    Reply, SkipReason, VisibleInventory,
};
use market_telegram_client::{ErrorDisposition, transport};
pub use worker::{BotInspection, TelegramWorker, inspect};
#[cfg(test)]
mod tests;
