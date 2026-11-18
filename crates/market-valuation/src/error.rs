//! Valuation failures and evidence; data-access errors remain owned by the data layer.
use market_actors::error::{DataError, DataErrorCode};
use market_gifts::{
    CurrencyId,
    valuation::{StrategyId, ValuationErrorDetails},
};
use std::sync::Arc;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValuationErrorCode {
    InvalidQuery,
    RestartRead,
    CapacityExceeded,
    Unavailable,
    NotFound,
    NotEnoughData,
    InvalidBaseline,
    UnsupportedStrategy,
    TimedOut,
}
#[derive(Debug, Clone)]
pub struct ValuationError {
    code: ValuationErrorCode,
    message: String,
    details: Option<Arc<ValuationErrorDetails>>,
    source: Option<DataError>,
}
pub type Result<T> = std::result::Result<T, ValuationError>;
impl ValuationError {
    fn new(code: ValuationErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
            source: None,
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ValuationErrorCode::InvalidQuery, message)
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ValuationErrorCode::Unavailable, message)
    }
    pub fn capacity(message: impl Into<String>) -> Self {
        Self::new(ValuationErrorCode::CapacityExceeded, message)
    }
    pub fn timed_out(message: impl Into<String>) -> Self {
        Self::new(ValuationErrorCode::TimedOut, message)
    }
    pub fn with_details(
        code: ValuationErrorCode,
        message: impl Into<String>,
        details: ValuationErrorDetails,
    ) -> Self {
        let mut error = Self::new(code, message);
        error.details = Some(Arc::new(details));
        error
    }
    pub fn with_context(mut self, strategy: StrategyId, currency: CurrencyId) -> Self {
        if self.details.is_none() {
            let mut details = ValuationErrorDetails::strategy(strategy);
            details.currency_id = Some(currency.clone());
            self.details = Some(Arc::new(details));
        }
        if let Some(details) = &mut self.details {
            let details = Arc::make_mut(details);
            if details.currency_id.is_none() {
                details.currency_id = Some(currency);
            }
        }
        self
    }
    pub fn code(&self) -> ValuationErrorCode {
        self.code
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn details(&self) -> Option<&Arc<ValuationErrorDetails>> {
        self.details.as_ref()
    }
}
impl From<DataError> for ValuationError {
    fn from(error: DataError) -> Self {
        let code = match error.code() {
            DataErrorCode::InvalidQuery => ValuationErrorCode::InvalidQuery,
            DataErrorCode::RestartRead => ValuationErrorCode::RestartRead,
            DataErrorCode::CapacityExceeded => ValuationErrorCode::CapacityExceeded,
            DataErrorCode::Unavailable => ValuationErrorCode::Unavailable,
            DataErrorCode::NotFound => ValuationErrorCode::NotFound,
            DataErrorCode::TimedOut => ValuationErrorCode::TimedOut,
        };
        Self {
            code,
            message: error.message().into(),
            details: None,
            source: Some(error),
        }
    }
}
impl std::fmt::Display for ValuationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ValuationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|source| source as _)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    #[test]
    fn data_failures_keep_their_source_and_valuation_context() {
        let error = ValuationError::from(DataError::capacity("read budget full"))
            .with_context(StrategyId::model_floor(), "GRAM".into());
        assert_eq!(error.code(), ValuationErrorCode::CapacityExceeded);
        assert_eq!(
            error
                .source()
                .unwrap()
                .downcast_ref::<DataError>()
                .unwrap()
                .code(),
            DataErrorCode::CapacityExceeded
        );
        assert_eq!(error.details().unwrap().strategy, StrategyId::model_floor());
        assert_eq!(
            error
                .details()
                .unwrap()
                .currency_id
                .as_ref()
                .unwrap()
                .as_str(),
            "GRAM"
        );
    }
}
