//! Errors exposed by market data actors, independent of RPC and HTTP frameworks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataErrorCode {
    InvalidQuery,
    RestartRead,
    CapacityExceeded,
    Unavailable,
    NotFound,
}
#[derive(Debug, Clone)]
pub struct DataError {
    code: DataErrorCode,
    message: String,
}
pub type Result<T> = std::result::Result<T, DataError>;
impl DataError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: DataErrorCode::InvalidQuery,
            message: message.into(),
        }
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: DataErrorCode::Unavailable,
            message: message.into(),
        }
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
        }
    }
}
