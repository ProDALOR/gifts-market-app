//! Shared typed actor-call error mapping.
#[cfg(feature = "portfolio")]
use crate::StrategyId;
use crate::ValuationError;
#[cfg(feature = "portfolio")]
use crate::{Handle, Mark};
#[cfg(feature = "portfolio")]
use market_gifts::CurrencyId;
#[cfg(feature = "portfolio")]
use std::sync::Arc;
#[cfg(feature = "portfolio")]
pub(crate) async fn invoke<K: Send + 'static, V: Send + Sync + 'static>(
    handle: &Handle<K, V, ValuationError>,
    request: K,
    strategy: StrategyId,
    currency: CurrencyId,
) -> Result<Arc<Mark<V>>, ValuationError> {
    let mark = handle.evaluate(request).await.map_err(|error| {
        let error = actor_error(error);
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

pub(crate) fn actor_error(error: crate::Error<ValuationError>) -> ValuationError {
    match error {
        crate::Error::Strategy(error) => error,
        crate::Error::Busy => ValuationError::capacity("Valuation requests are busy"),
        crate::Error::Stopped => ValuationError::unavailable("Valuation service is unavailable"),
        crate::Error::TimedOut => ValuationError::timed_out("Valuation request timed out"),
    }
}
