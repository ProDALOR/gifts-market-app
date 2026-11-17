//! Gift-market Telegram commands and presentation over the market actor layer.
mod commands;
mod service;
pub use commands::{Command, CommandKind, CommandRequest, FloorQuery, MarketCommands};
pub use service::{BotService, configure};
#[cfg(test)]
mod tests;
