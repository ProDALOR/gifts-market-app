use super::{Reply, render};
use market_portfolio::{
    InventoryError, PortfolioError, PortfolioHandle, PortfolioRequest, Priority, UserId,
};
use market_telegram::BotError;
pub(super) async fn execute(
    service: &PortfolioHandle,
    user: u64,
    currency: market_gifts::CurrencyId,
    strategy: market_gifts::valuation::StrategyId,
) -> Result<Reply, BotError> {
    let user = UserId::new(user).map_err(|_| BotError::InvalidResponse)?;
    match service
        .request(PortfolioRequest {
            strategy,
            user,
            currency_id: currency,
            priority: Priority::Interactive,
        })
        .await
    {
        Ok(portfolio) => Ok(render::portfolio(
            &portfolio.valuation,
            &portfolio.inventory,
        )),
        Err(PortfolioError::Inventory(InventoryError::Authorization)) => {
            Err(BotError::Authorization)
        }
        Err(PortfolioError::Inventory(error)) => Ok(render::inventory_error(error)),
        Err(PortfolioError::Valuation(error)) => Ok(render::error_reply(error)),
        Err(PortfolioError::Busy) => Ok(Reply {
            text: "Portfolio requests are busy. Please try again shortly.".into(),
        }),
        Err(PortfolioError::TimedOut) => Ok(Reply {
            text: "Your portfolio read timed out. Please try again shortly.".into(),
        }),
        Err(_) => Ok(Reply {
            text: "Portfolio service is unavailable. Please try again shortly.".into(),
        }),
    }
}
