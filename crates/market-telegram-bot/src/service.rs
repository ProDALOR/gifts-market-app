//! Bot application assembly; process supervision stays in market-app.
use crate::commands::{LaunchCommands, MarketCommands};
use market_actors::MarketDataHandle;
use market_telegram::{BotConfig, BotError, BotUser, TelegramWorker};
use tokio_util::sync::CancellationToken;
pub struct BotService {
    worker: TelegramWorker,
    commands: MarketCommands,
}
impl BotService {
    pub fn new(
        worker: TelegramWorker,
        data: MarketDataHandle,
        portfolio: market_portfolio::PortfolioHandle,
        valuation: market_valuation::ValuationHandle,
    ) -> Self {
        Self {
            worker,
            commands: MarketCommands::new(data, portfolio, valuation),
        }
    }
    pub async fn run_with_invocations(
        self,
        stop: CancellationToken,
        invocations: market_invocations::Handle,
    ) -> Result<(), BotError> {
        self.worker
            .run_with_invocations(&self.commands, stop, invocations)
            .await
    }
    pub async fn run(self, stop: CancellationToken) -> Result<(), BotError> {
        self.worker.run(self.commands, stop).await
    }
}
pub async fn configure(config: BotConfig) -> anyhow::Result<BotUser> {
    Ok(TelegramWorker::new(config)?
        .configure(&LaunchCommands)
        .await?)
}
