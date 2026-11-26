//! Gift-market Telegram commands and presentation over the market actor layer.
mod commands;
mod icons;
pub use icons::{ReportIcon, ReportIcons};
mod service;
pub use commands::{
    BotCurrency, Command, CommandKind, CommandRequest, GridMode, MarketCommands, PortfolioAction,
    PortfolioMode, PortfolioRequest, ValueRequest,
};
pub use service::{BotService, configure};
#[cfg(test)]
mod tests;

pub use commands::sharing::{ShareError, SharedMessage, Sharing};
