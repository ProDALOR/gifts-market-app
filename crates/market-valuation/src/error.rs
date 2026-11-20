//! Valuation failures and evidence; data-access errors remain owned by the data layer.
use crate::{StrategyId, ValuationErrorDetails};
use market_gifts::CurrencyId;
use std::sync::Arc;
use tonic::{Code, Status};
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationErrorCode {
    InvalidQuery,
    RestartRead,
    CapacityExceeded,
    Unavailable,
    NotFound,
    NotEnoughData,
    InvalidBaseline,
    UnsupportedStrategy,
    UnsupportedTarget,
    TimedOut,
}
#[derive(Debug, Clone)]
pub struct ValuationError {
    code: ValuationErrorCode,
    message: String,
    details: Option<Arc<ValuationErrorDetails>>,
    source: Option<Status>,
}
#[cfg(feature = "portfolio")]
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
    pub fn unsupported_target(message: impl Into<String>) -> Self {
        Self::new(ValuationErrorCode::UnsupportedTarget, message)
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
impl From<Status> for ValuationError {
    fn from(error: Status) -> Self {
        let code = match error.code() {
            Code::InvalidArgument => ValuationErrorCode::InvalidQuery,
            Code::FailedPrecondition | Code::OutOfRange => ValuationErrorCode::RestartRead,
            Code::ResourceExhausted => ValuationErrorCode::CapacityExceeded,
            Code::NotFound => ValuationErrorCode::NotFound,
            _ => ValuationErrorCode::Unavailable,
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
        let error = ValuationError::from(Status::resource_exhausted("read budget full"))
            .with_context(
                StrategyId::new(<crate::GiftFloor as crate::Strategy>::ID).unwrap(),
                "GRAM".into(),
            );
        assert_eq!(error.code(), ValuationErrorCode::CapacityExceeded);
        assert_eq!(
            error
                .source()
                .unwrap()
                .downcast_ref::<Status>()
                .unwrap()
                .code(),
            Code::ResourceExhausted
        );
        assert_eq!(
            error.details().unwrap().strategy,
            StrategyId::new(<crate::GiftFloor as crate::Strategy>::ID).unwrap()
        );
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
