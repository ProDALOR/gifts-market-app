//! Typed command requests, services and reply rendering.
mod floor;
mod parse;
mod portfolio;
mod render;
use super::{BotError, GiftSource};
use market_actors::MarketDataHandle;
pub use parse::CommandKind;
pub(super) use parse::{Incoming, parse};
pub(super) use render::launch_reply;
use std::future::Future;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteCurrency(String);
impl QuoteCurrency {
    /// Normalize a quote symbol; catalog availability is checked by the market actors.
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() > 128 || text.split_whitespace().count() > 1 {
            return None;
        }
        Some(Self(match text.trim().to_ascii_uppercase().as_str() {
            "" | "TON" => "GRAM".into(),
            value => value.into(),
        }))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorQuery {
    pub collection: String,
    pub model: String,
    pub currency: QuoteCurrency,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Start,
    Market,
    Help,
    Status,
    Floor(FloorQuery),
    Portfolio(QuoteCurrency),
    Invalid(CommandKind),
}
impl Command {
    pub fn kind(&self) -> CommandKind {
        match self {
            Self::Start => CommandKind::Start,
            Self::Market => CommandKind::Market,
            Self::Help => CommandKind::Help,
            Self::Status => CommandKind::Status,
            Self::Floor(_) => CommandKind::Floor,
            Self::Portfolio(_) => CommandKind::Portfolio,
            Self::Invalid(kind) => *kind,
        }
    }
}
/// The worker obtains these identifiers from the incoming private message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRequest {
    pub command: Command,
    pub user_id: u64,
    pub message_id: i32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
}
impl Reply {
    pub(super) fn bounded(mut self) -> Self {
        if self.text.trim().is_empty() {
            self.text = "This command produced no result.".into();
        }
        if self.text.encode_utf16().count() > 4096 {
            let mut units = 0;
            self.text = self
                .text
                .chars()
                .take_while(|ch| {
                    units += ch.len_utf16();
                    units <= 4095
                })
                .collect();
            self.text.push('…');
        }
        self
    }
}
pub trait CommandHandler: Send + Sync {
    fn handle<S: GiftSource>(
        &self,
        request: CommandRequest,
        gifts: &S,
    ) -> impl Future<Output = Result<Reply, BotError>> + Send;
}
pub struct MarketCommands {
    market: MarketDataHandle,
}
impl MarketCommands {
    pub fn new(market: MarketDataHandle) -> Self {
        Self { market }
    }
}
impl CommandHandler for MarketCommands {
    async fn handle<S: GiftSource>(
        &self,
        request: CommandRequest,
        gifts: &S,
    ) -> Result<Reply, BotError> {
        match request.command {
            Command::Status => Ok(match self.market.status().await {
                Ok(status) => render::status_reply(status),
                Err(error) => render::error_reply(error),
            }),
            Command::Floor(query) => Ok(floor::execute(&self.market, query).await),
            Command::Portfolio(currency) => portfolio::execute(&self.market, gifts, currency).await,
            command => {
                LaunchCommands
                    .handle(CommandRequest { command, ..request }, gifts)
                    .await
            }
        }
    }
}
pub(in crate::bot) struct LaunchCommands;
impl CommandHandler for LaunchCommands {
    async fn handle<S: GiftSource>(
        &self,
        request: CommandRequest,
        _: &S,
    ) -> Result<Reply, BotError> {
        if matches!(request.command, Command::Invalid(_)) {
            return Ok(render::usage(request.command.kind()));
        }
        launch_reply(request.command.kind()).ok_or(BotError::Configuration(
            "Market data handler is required for this command",
        ))
    }
}
