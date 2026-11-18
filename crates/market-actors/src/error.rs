//! Errors exposed by market data actors, independent of RPC and HTTP frameworks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataErrorCode {
    InvalidQuery,
    RestartRead,
    CapacityExceeded,
    Unavailable,
    NotFound,
    NotEnoughData,
    UnsupportedStrategy,
    TimedOut,
}
#[derive(Debug, Clone)]
pub struct DataError {
    code: DataErrorCode,
    message: String,
    valuation: Option<std::sync::Arc<market_gifts::valuation::ValuationErrorDetails>>,
}
pub type Result<T> = std::result::Result<T, DataError>;
impl DataError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: DataErrorCode::InvalidQuery,
            message: message.into(),
            valuation: None,
        }
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: DataErrorCode::Unavailable,
            message: message.into(),
            valuation: None,
        }
    }
    pub fn capacity(message: impl Into<String>) -> Self {
        Self {
            code: DataErrorCode::CapacityExceeded,
            message: message.into(),
            valuation: None,
        }
    }
    pub fn timed_out(message: impl Into<String>) -> Self {
        Self {
            code: DataErrorCode::TimedOut,
            message: message.into(),
            valuation: None,
        }
    }
    pub fn valuation(
        code: DataErrorCode,
        message: impl Into<String>,
        details: market_gifts::valuation::ValuationErrorDetails,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            valuation: Some(std::sync::Arc::new(details)),
        }
    }
    pub fn valuation_details(
        &self,
    ) -> Option<&std::sync::Arc<market_gifts::valuation::ValuationErrorDetails>> {
        self.valuation.as_ref()
    }
    pub fn with_valuation(
        mut self,
        strategy: market_gifts::valuation::StrategyId,
        currency: market_gifts::CurrencyId,
    ) -> Self {
        if self.valuation.is_none() {
            let mut context = market_gifts::valuation::ValuationErrorDetails::strategy(strategy);
            context.currency_id = Some(currency);
            self.valuation = Some(std::sync::Arc::new(context));
        }
        self
    }
    pub fn code(&self) -> DataErrorCode {
        self.code
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}
impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for DataError {}
impl From<tonic::Status> for DataError {
    fn from(error: tonic::Status) -> Self {
        let code = match error.code() {
            tonic::Code::InvalidArgument => DataErrorCode::InvalidQuery,
            tonic::Code::NotFound => DataErrorCode::NotFound,
            tonic::Code::FailedPrecondition | tonic::Code::OutOfRange => DataErrorCode::RestartRead,
            tonic::Code::ResourceExhausted => DataErrorCode::CapacityExceeded,
            _ => DataErrorCode::Unavailable,
        };
        Self {
            code,
            message: error.message().to_owned(),
            valuation: None,
        }
    }
}
