//! Typed command requests, services and reply rendering.
use market_links::MarketSelection;
mod browse;
mod currency;
mod history;
pub use currency::BotCurrency;
mod portfolio;
pub use portfolio::{PortfolioAction, PortfolioMode, PortfolioRequest};
mod gallery;
mod interactions;
mod layout;
mod orders;
pub use layout::GridMode;
mod parse;
mod render;
mod report;
mod rich;
pub mod sharing;
mod views;
use market_actors::MarketDataHandle;
use market_telegram::{BotError, IncomingMessage, MenuCommand, MessageHandler, Reply};
pub use parse::CommandKind;
pub(crate) use parse::parse;
use std::future::Future;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Start,
    Browse(MarketSelection),
    Help,
    Status,
    Gift(ValueRequest),
    GiftHelp,
    View(u64),
    MarketView(u64),
    Shared(u64),
    Portfolio(PortfolioRequest),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueRequest {
    pub gift: market_gifts::GiftSlug,
    pub currency_id: market_gifts::CurrencyId,
}
/// Identity comes from the authenticated transport event.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandRequest {
    pub command: Command,
    pub user_id: u64,
    pub message_id: i32,
}
#[derive(Clone)]
pub struct MarketCommands {
    shares: std::sync::Arc<interactions::ViewCache<sharing::SharedView>>,
    public_url: Option<url::Url>,
    portfolio: Option<market_portfolio::PortfolioHandle>,
    portfolio_reads: std::sync::Arc<market_portfolio::PortfolioReadCache>,
    market: MarketDataHandle,
    valuation: market_valuation::ValuationHandle,
    views: std::sync::Arc<interactions::Views>,
    markets: std::sync::Arc<interactions::ViewCache<browse::View>>,
    icons: std::sync::Arc<crate::ReportIcons>,
}
impl MarketCommands {
    pub fn new(market: MarketDataHandle, valuation: market_valuation::ValuationHandle) -> Self {
        Self {
            shares: std::sync::Arc::new(interactions::ViewCache::new("s", 128)),
            public_url: None,
            market,
            valuation,
            portfolio: None,
            portfolio_reads: Default::default(),
            views: Default::default(),
            markets: std::sync::Arc::new(interactions::ViewCache::new("m", 64)),
            icons: Default::default(),
        }
    }
    pub fn with_public_url(mut self, url: Option<url::Url>) -> Self {
        self.public_url = url;
        self
    }
    pub fn with_portfolio(mut self, portfolio: market_portfolio::PortfolioHandle) -> Self {
        self.set_portfolio(portfolio);
        self
    }
    pub(crate) fn set_portfolio(&mut self, portfolio: market_portfolio::PortfolioHandle) {
        self.portfolio = Some(portfolio);
    }
    /// Configure presentation once for private, inline, guest and callback reports.
    pub fn with_report_icons(mut self, icons: crate::ReportIcons) -> Self {
        self.set_report_icons(icons);
        self
    }
    pub(crate) fn set_report_icons(&mut self, icons: crate::ReportIcons) {
        self.icons = std::sync::Arc::new(icons);
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
            Command::Shared(id) => Ok(self.shared_reply(request.user_id, id).await),
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
            Command::Browse(filters) => Ok(self.browse(filters).await),
            Command::Gift(query) => Ok(self.value(query).await),
            Command::View(id) => Ok(self.view(id).await),
            Command::MarketView(id) => Ok(self.market_view(id).await),
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
