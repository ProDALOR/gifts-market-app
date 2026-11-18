//! Typed command requests, services and reply rendering.
mod floor;
mod parse;
mod portfolio;
mod render;
use market_actors::MarketDataHandle;
use market_telegram::{BotError, IncomingMessage, MenuCommand, MessageHandler, Reply};
pub use parse::CommandKind;
pub(crate) use parse::parse;
pub(super) use render::launch_reply;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorQuery {
    pub model: market_gifts::valuation::ModelLookup,
    pub currency: market_gifts::CurrencyId,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Start,
    Market,
    Help,
    Status,
    Floor(FloorQuery),
    Portfolio(market_gifts::CurrencyId),
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
pub struct MarketCommands {
    market: MarketDataHandle,
    portfolio: market_portfolio::PortfolioHandle,
    valuation: market_actors::valuation::ValuationHandle,
}
impl MarketCommands {
    pub fn new(
        market: MarketDataHandle,
        portfolio: market_portfolio::PortfolioHandle,
        valuation: market_actors::valuation::ValuationHandle,
    ) -> Self {
        Self {
            market,
            portfolio,
            valuation,
        }
    }
}
impl MessageHandler for MarketCommands {
    type Request = CommandRequest;
    fn commands(&self) -> Vec<MenuCommand> {
        menu()
    }
    fn launch_label(&self) -> &'static str {
        "Open market"
    }
    fn parse(&self, message: IncomingMessage<'_>, usernames: &[String]) -> Option<CommandRequest> {
        parse(message, usernames)
    }
    async fn handle(&self, request: CommandRequest) -> Result<Reply, BotError> {
        match request.command {
            Command::Status => Ok(match self.market.status().await {
                Ok(status) => render::status_reply(status),
                Err(error) => render::error_reply(error),
            }),
            Command::Floor(query) => Ok(floor::execute(&self.valuation, query).await),
            Command::Portfolio(currency) => {
                portfolio::execute(&self.portfolio, request.user_id, currency).await
            }
            command => {
                LaunchCommands
                    .handle(CommandRequest { command, ..request })
                    .await
            }
        }
    }
}
pub(crate) struct LaunchCommands;
impl MessageHandler for LaunchCommands {
    type Request = CommandRequest;
    fn commands(&self) -> Vec<MenuCommand> {
        menu()
    }
    fn launch_label(&self) -> &'static str {
        "Open market"
    }
    fn parse(&self, message: IncomingMessage<'_>, usernames: &[String]) -> Option<CommandRequest> {
        parse(message, usernames)
    }
    async fn handle(&self, request: CommandRequest) -> Result<Reply, BotError> {
        if matches!(request.command, Command::Invalid(_)) {
            return Ok(render::usage(request.command.kind()));
        }
        launch_reply(request.command.kind()).ok_or(BotError::Configuration(
            "Market data handler is required for this command",
        ))
    }
}

fn menu() -> Vec<MenuCommand> {
    CommandKind::ALL
        .iter()
        .map(|kind| MenuCommand {
            name: kind.name(),
            description: kind.description(),
        })
        .collect()
}
