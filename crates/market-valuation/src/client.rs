//! Shared typed actor-call error mapping.
use crate::StrategyId;
use crate::{Handle, Mark, ValuationError};
use market_gifts::CurrencyId;
use std::sync::Arc;
pub(crate) async fn invoke<K: Send + 'static, V: Send + Sync + 'static>(
    handle: &Handle<K, V, ValuationError>,
    request: K,
    strategy: StrategyId,
    currency: CurrencyId,
) -> Result<Arc<Mark<V>>, ValuationError> {
    let mark = handle.evaluate(request).await.map_err(|error| {
        let error = match error {
            crate::Error::Strategy(e) => e,
            crate::Error::Busy => ValuationError::capacity("Valuation requests are busy"),
            crate::Error::Stopped => {
                ValuationError::unavailable("Valuation service is unavailable")
            }
            crate::Error::TimedOut => ValuationError::timed_out("Valuation request timed out"),
        };
        error.with_context(strategy.clone(), currency.clone())
    })?;
    if mark.strategy != strategy.as_str() {
        return Err(ValuationError::unavailable(
            "Valuation strategy returned an inconsistent mark",
        )
        .with_context(strategy, currency));
    }
    Ok(mark)
}
