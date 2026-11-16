//! Supervised bot commands over a private MTProto adapter.
mod commands;
mod config;
mod dispatcher;
mod error;
mod inventory;
mod mtproto;
mod transport;
mod worker;
pub use commands::{
    Command, CommandHandler, CommandKind, CommandRequest, FloorQuery, MarketCommands,
    QuoteCurrency, Reply,
};
pub use config::{BotConfig, BotCredentials};
pub use error::BotError;
pub use inventory::{
    GiftRelation, GiftSource, InventoryCompletion, InventoryError, PartialReason, ProfileGift,
    SkipReason, VisibleInventory,
};
pub use transport::BotUser;
pub use worker::{BotInspection, BotWorker, configure, inspect};
#[cfg(test)]
mod tests;
