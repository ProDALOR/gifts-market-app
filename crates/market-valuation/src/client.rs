//! Shared typed actor-call error mapping.
use crate::ValuationError;

pub(crate) fn actor_error(error: crate::Error<ValuationError>) -> ValuationError {
    match error {
        crate::Error::Strategy(error) => error,
        crate::Error::Busy => ValuationError::capacity("Valuation requests are busy"),
        crate::Error::Stopped => ValuationError::unavailable("Valuation service is unavailable"),
        crate::Error::TimedOut => ValuationError::timed_out("Valuation request timed out"),
    }
}
