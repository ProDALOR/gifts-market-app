//! Typed command requests, services and reply rendering.
use market_links::MarketSelection;
mod app_links;
mod browse;
mod cache;
mod reports;
pub use reports::ReportCallback;
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

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Start,
    Browse(MarketSelection),
    Help,
    Status,
    Gift(ValueRequest),
    GiftHelp,
    Report(ReportCallback),
    PortfolioTarget(sharing::PortfolioTarget),
    CallbackDenied,
    CallbackExpired,
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
    /// Inline/guest replies bind their controls to the authenticated requesting user.
    pub inline: bool,
}
#[derive(Clone)]
pub struct MarketCommands {
    public_url: Option<url::Url>,
    portfolio: Option<market_portfolio::PortfolioHandle>,
    portfolio_reads: std::sync::Arc<market_portfolio::PortfolioReadCache>,
    market: MarketDataHandle,
    valuation: market_valuation::ValuationHandle,
    reports: std::sync::Arc<reports::Reports>,
    icons: std::sync::Arc<crate::ReportIcons>,
}
impl MarketCommands {
    pub fn new(market: MarketDataHandle, valuation: market_valuation::ValuationHandle) -> Self {
        Self {
            public_url: None,
            market,
            valuation,
            portfolio: None,
            portfolio_reads: Default::default(),
            reports: Default::default(),
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
        parse::parse_with_origin(message, usernames, self.public_url.as_ref())
    }
    fn parse_event(
        &self,
        event: &market_telegram::Event,
        usernames: &[String],
    ) -> Option<CommandRequest> {
        if matches!(event, market_telegram::Event::Callback { .. }) {
            self.reports.parse(event, usernames)
        } else {
            interactions::parse_event_with_origin(event, usernames, self.public_url.as_ref())
        }
    }
    async fn handle_with_progress(
        &self,
        request: CommandRequest,
        updates: tokio::sync::watch::Sender<Reply>,
    ) -> Result<Reply, BotError> {
        if request.inline {
            return self.handle(request).await;
        }
        if let Command::Portfolio(query) = request.command {
            Ok(self.reports.reply(
                request.user_id,
                self.portfolio_reply(query, Some(updates)).await,
            ))
        } else {
            self.handle(request).await
        }
    }
    async fn handle(&self, request: CommandRequest) -> Result<Reply, BotError> {
        let reply = match request.command {
            Command::PortfolioTarget(target) => Ok(self.portfolio_target(target).await),
            Command::CallbackDenied => Ok(Reply::notice(
                "Only the person who created this report can use its controls.",
            )),
            Command::CallbackExpired => Ok(Reply::notice(
                "These controls have expired. Create a new inline report.",
            )),
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
            Command::Report(callback) => Ok(self
                .report_request(callback, request.user_id, request.inline)
                .await),
            command => {
                LaunchCommands
                    .handle(CommandRequest { command, ..request })
                    .await
            }
        }?;
        Ok(self.reports.reply(request.user_id, reply))
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
        render::launch_reply(&request.command, request.user_id).ok_or(BotError::Configuration(
            "Market data handler is required for this request",
        ))
    }
}

fn menu() -> Vec<MenuCommand> {
    CommandKind::PUBLIC
        .iter()
        .map(|kind| MenuCommand {
            name: kind.name(),
            description: kind.description(),
        })
        .collect()
}

#[cfg(test)]
mod tests;
