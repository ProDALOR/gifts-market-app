//! Gift-market Telegram commands and presentation over the market actor layer.
mod commands;
mod service;
pub use commands::{Command, CommandKind, CommandRequest, MarketCommands, ValueRequest};
pub use service::{BotService, configure};
#[cfg(test)]
mod tests;
