//! Supervised Telegram update delivery and application command dispatch.
mod dispatcher;
mod handler;
mod worker;
pub use handler::MessageHandler;
use market_telegram_client::ErrorDisposition;
pub use market_telegram_client::interactions::{CallbackTarget, Event, InlineMessageId, Lane};
pub use market_telegram_client::transport;
pub use market_telegram_client::{
    BotConfig, BotCredentials, BotError, BotUser, IncomingMessage, InventoryClient, LaunchButton,
    MenuCommand, Reply, ReplyButton,
};
pub use worker::{BotInspection, TelegramWorker, inspect};
#[cfg(test)]
mod tests;
