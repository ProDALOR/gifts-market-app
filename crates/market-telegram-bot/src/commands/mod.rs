//! Typed command requests, services and reply rendering.
mod history;
mod portfolio;
pub use portfolio::PortfolioRequest;
mod interactions;
mod parse;
mod render;
mod report;
mod views;
use market_actors::MarketDataHandle;
use market_telegram::{BotError, IncomingMessage, MenuCommand, MessageHandler, Reply};
pub use parse::CommandKind;
pub(crate) use parse::parse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Start,
    Market,
    Help,
    Status,
    Gift(ValueRequest),
    GiftHelp,
    View(u64),
    Portfolio(PortfolioRequest),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueRequest {
    pub gift: market_gifts::GiftSlug,
    pub currency_id: market_gifts::CurrencyId,
}
/// Identity comes from the authenticated transport event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRequest {
    pub command: Command,
    pub user_id: u64,
    pub message_id: i32,
}
pub struct MarketCommands {
    portfolio: Option<market_portfolio::PortfolioHandle>,
    market: MarketDataHandle,
    valuation: market_valuation::ValuationHandle,
    views: interactions::Views,
    icons: std::sync::Arc<crate::ReportIcons>,
}
impl MarketCommands {
    pub fn new(market: MarketDataHandle, valuation: market_valuation::ValuationHandle) -> Self {
        Self {
            market,
            valuation,
            portfolio: None,
            views: Default::default(),
            icons: Default::default(),
        }
    }
    pub fn with_portfolio(mut self, portfolio: market_portfolio::PortfolioHandle) -> Self {
        self.portfolio = Some(portfolio);
        self
    }
    /// Configure presentation once for private, inline, guest and callback reports.
    pub fn with_report_icons(mut self, icons: crate::ReportIcons) -> Self {
        self.icons = std::sync::Arc::new(icons);
        self
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
    fn parse_event(
        &self,
        event: &market_telegram::Event,
        usernames: &[String],
    ) -> Option<CommandRequest> {
        interactions::parse_event(event, usernames)
    }
    async fn handle_with_progress(
        &self,
        request: CommandRequest,
        updates: tokio::sync::watch::Sender<Reply>,
    ) -> Result<Reply, BotError> {
        if let Command::Portfolio(query) = request.command {
            Ok(self.portfolio_reply(query, Some(updates)).await)
        } else {
            self.handle(request).await
        }
    }
    async fn handle(&self, request: CommandRequest) -> Result<Reply, BotError> {
        match request.command {
            Command::Status => Ok(match self.market.status().await {
                Ok(status) => render::status_reply(status),
                Err(error) => render::error_reply(
                    if error.code() == market_actors::error::DataErrorCode::Unavailable {
                        "Market data is temporarily unavailable"
                    } else {
                        error.message()
                    },
                ),
            }),
            Command::Portfolio(query) => Ok(self.portfolio_reply(query, None).await),
            Command::Gift(query) => Ok(self.value(query).await),
            Command::View(id) => Ok(self.view(id).await),
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
        render::launch_reply(&request.command).ok_or(BotError::Configuration(
            "Market data handler is required for this request",
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

#[cfg(test)]
mod tests;
