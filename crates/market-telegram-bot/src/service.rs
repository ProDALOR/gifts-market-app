//! Bot application assembly; process supervision stays in market-app.
use crate::commands::{LaunchCommands, MarketCommands};
use market_actors::MarketDataHandle;
use market_telegram::{BotConfig, BotError, BotUser, TelegramWorker};
use tokio_util::sync::CancellationToken;
pub struct BotService {
    worker: TelegramWorker,
    commands: std::sync::Arc<MarketCommands>,
}
impl BotService {
    pub fn new(
        worker: TelegramWorker,
        data: MarketDataHandle,
        valuation: market_valuation::ValuationHandle,
    ) -> Self {
        let public_url = worker.public_url().cloned();
        Self {
            worker,
            commands: std::sync::Arc::new(
                MarketCommands::new(data, valuation).with_public_url(public_url),
            ),
        }
    }
    pub fn with_portfolio(mut self, portfolio: market_portfolio::PortfolioHandle) -> Self {
        std::sync::Arc::make_mut(&mut self.commands).set_portfolio(portfolio);
        self
    }
    pub fn with_report_icons(mut self, icons: crate::ReportIcons) -> Self {
        std::sync::Arc::make_mut(&mut self.commands).set_report_icons(icons);
        self
    }
    pub async fn run_with_invocations(
        self,
        stop: CancellationToken,
        invocations: market_invocations::Handle,
    ) -> Result<(), BotError> {
        self.worker
            .run_with_invocations(self.commands.as_ref(), stop, invocations)
            .await
    }
    pub async fn run(self, stop: CancellationToken) -> Result<(), BotError> {
        let invocations =
            market_invocations::Runtime::start(Default::default()).map_err(|_| BotError::Runner)?;
        let result = self.run_with_invocations(stop, invocations.handle()).await;
        invocations.shutdown().await;
        result
    }
}
pub async fn configure(config: BotConfig) -> anyhow::Result<BotUser> {
    Ok(TelegramWorker::new(config)?
        .configure(&LaunchCommands)
        .await?)
}
