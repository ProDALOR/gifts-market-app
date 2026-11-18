use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthorized,
    Forbidden,
    PayloadTooLarge,
    UnsupportedMediaType,
    MethodNotAllowed,
    Internal,
    InvalidQuery,
    RestartRead,
    CapacityExceeded,
    Unavailable,
    NotFound,
    NotEnoughData,
    UnsupportedStrategy,
    TimedOut,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<std::sync::Arc<market_gifts::valuation::ValuationErrorDetails>>,
}
pub type Result<T> = std::result::Result<T, ApiError>;
impl ApiError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InvalidQuery,
            message: message.into(),
            details: None,
        }
    }
}
impl From<market_actors::error::DataError> for ApiError {
    fn from(error: market_actors::error::DataError) -> Self {
        use market_actors::error::DataErrorCode;
        let code = match error.code() {
            DataErrorCode::InvalidQuery => ErrorCode::InvalidQuery,
            DataErrorCode::NotFound => ErrorCode::NotFound,
            DataErrorCode::RestartRead => ErrorCode::RestartRead,
            DataErrorCode::CapacityExceeded => ErrorCode::CapacityExceeded,
            DataErrorCode::Unavailable => ErrorCode::Unavailable,
            DataErrorCode::NotEnoughData => ErrorCode::NotEnoughData,
            DataErrorCode::UnsupportedStrategy => ErrorCode::UnsupportedStrategy,
            DataErrorCode::TimedOut => ErrorCode::TimedOut,
        };
        let message = if matches!(code, ErrorCode::Unavailable) {
            "Market data service is unavailable".into()
        } else {
            error.message().to_owned()
        };
        Self {
            code,
            message,
            details: error.valuation_details().cloned(),
        }
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
            ErrorCode::Forbidden => StatusCode::FORBIDDEN,
            ErrorCode::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ErrorCode::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::InvalidQuery => StatusCode::BAD_REQUEST,
            ErrorCode::RestartRead => StatusCode::CONFLICT,
            ErrorCode::CapacityExceeded => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::NotEnoughData => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::UnsupportedStrategy => StatusCode::BAD_REQUEST,
            ErrorCode::TimedOut => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(ErrorResponse {
            error: self.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: ApiError,
}

#[cfg(feature = "miniapp")]
impl From<market_miniapp::auth::AuthError> for ApiError {
    fn from(error: market_miniapp::auth::AuthError) -> Self {
        Self {
            details: None,
            code: ErrorCode::Unauthorized,
            message: error.to_string(),
        }
    }
}

impl ApiError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Unavailable,
            message: message.into(),
            details: None,
        }
    }
    pub fn capacity(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::CapacityExceeded,
            message: message.into(),
            details: None,
        }
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Forbidden,
            message: message.into(),
            details: None,
        }
    }
    pub fn not_found() -> Self {
        Self {
            details: None,
            code: ErrorCode::NotFound,
            message: "Route not found".into(),
        }
    }
}
pub(crate) fn configure_extractors(cfg: &mut actix_web::web::ServiceConfig, limit: usize) {
    use actix_web::{error::JsonPayloadError, web};
    cfg.app_data(
        web::JsonConfig::default()
            .limit(limit)
            .error_handler(|error, _| {
                match error {
                    JsonPayloadError::Overflow { .. }
                    | JsonPayloadError::OverflowKnownLength { .. } => ApiError {
                        details: None,
                        code: ErrorCode::PayloadTooLarge,
                        message: "JSON payload exceeds the request limit".into(),
                    },
                    JsonPayloadError::ContentType => ApiError {
                        details: None,
                        code: ErrorCode::UnsupportedMediaType,
                        message: "Expected application/json".into(),
                    },
                    _ => ApiError::invalid("invalid JSON query"),
                }
                .into()
            }),
    )
    .app_data(
        web::QueryConfig::default()
            .error_handler(|_, _| ApiError::invalid("invalid query parameters").into()),
    )
    .app_data(
        web::PathConfig::default()
            .error_handler(|_, _| ApiError::invalid("invalid path parameters").into()),
    );
}
/// Normalize framework/CORS failures too, preserving headers such as Allow and CORS.
pub(crate) fn normalize<B: 'static>(
    response: actix_web::dev::ServiceResponse<B>,
) -> actix_web::Result<actix_web::middleware::ErrorHandlerResponse<B>> {
    use actix_web::{http::header, middleware::ErrorHandlerResponse};
    if response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"))
    {
        return Ok(ErrorHandlerResponse::Response(
            response.map_into_left_body(),
        ));
    }
    let status = response.status();
    let code = match status.as_u16() {
        401 => ErrorCode::Unauthorized,
        403 => ErrorCode::Forbidden,
        404 => ErrorCode::NotFound,
        405 => ErrorCode::MethodNotAllowed,
        413 => ErrorCode::PayloadTooLarge,
        415 => ErrorCode::UnsupportedMediaType,
        429 => ErrorCode::CapacityExceeded,
        503 => ErrorCode::Unavailable,
        500..=599 => ErrorCode::Internal,
        _ => ErrorCode::InvalidQuery,
    };
    let error = ApiError {
        details: None,
        code,
        message: status.canonical_reason().unwrap_or("Request failed").into(),
    };
    let mut builder = HttpResponse::build(status);
    for (name, value) in response.headers() {
        if name != header::CONTENT_TYPE
            && name != header::CONTENT_LENGTH
            && name != header::CONTENT_ENCODING
        {
            builder.append_header((name.clone(), value.clone()));
        }
    }
    let (request, _) = response.into_parts();
    Ok(ErrorHandlerResponse::Response(
        actix_web::dev::ServiceResponse::new(request, builder.json(ErrorResponse { error }))
            .map_into_right_body(),
    ))
}
