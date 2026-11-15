use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthorized,
    InvalidQuery,
    RestartRead,
    CapacityExceeded,
    Unavailable,
    NotFound,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}
pub type Result<T> = std::result::Result<T, ApiError>;
impl ApiError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InvalidQuery,
            message: message.into(),
        }
    }
}
impl From<tonic::Status> for ApiError {
    fn from(error: tonic::Status) -> Self {
        let code = match error.code() {
            tonic::Code::InvalidArgument => ErrorCode::InvalidQuery,
            tonic::Code::NotFound => ErrorCode::NotFound,
            tonic::Code::FailedPrecondition | tonic::Code::OutOfRange => ErrorCode::RestartRead,
            tonic::Code::ResourceExhausted => ErrorCode::CapacityExceeded,
            _ => ErrorCode::Unavailable,
        };
        let message = if matches!(code, ErrorCode::Unavailable) {
            "Market data service is unavailable".into()
        } else {
            error.message().to_owned()
        };
        Self { code, message }
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ApiError {}
impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        match self.code {
            ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorCode::InvalidQuery => StatusCode::BAD_REQUEST,
            ErrorCode::RestartRead => StatusCode::CONFLICT,
            ErrorCode::CapacityExceeded => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
        }
    }
    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(serde_json::json!({"error":self}))
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: ApiError,
}

impl From<market_telegram_app::auth::AuthError> for ApiError {
    fn from(error: market_telegram_app::auth::AuthError) -> Self {
        Self {
            code: ErrorCode::Unauthorized,
            message: error.to_string(),
        }
    }
}
