use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidQuery,
    InvalidCursor,
    CursorMismatch,
    CursorExpired,
    ResyncRequired,
    CapacityExceeded,
    ReplicaUnavailable,
    NotFound,
    ReadOnly,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: ApiError,
}

pub type Result<T> = std::result::Result<T, ApiError>;

impl ApiError {
    pub fn new(code: ErrorCode, message: impl std::fmt::Display) -> Self {
        Self {
            code,
            message: message.to_string(),
            field: None,
        }
    }
    pub fn invalid_query(message: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::InvalidQuery, message)
    }
    pub fn field(field: &str, message: impl std::fmt::Display) -> Self {
        Self {
            code: ErrorCode::InvalidQuery,
            message: message.to_string(),
            field: Some(field.into()),
        }
    }
    pub fn unavailable() -> Self {
        Self::new(
            ErrorCode::ReplicaUnavailable,
            "local replica has no usable snapshot",
        )
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(&self.message)
    }
}
impl std::error::Error for ApiError {}
impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        match self.code {
            ErrorCode::InvalidQuery | ErrorCode::InvalidCursor | ErrorCode::CursorMismatch => {
                StatusCode::BAD_REQUEST
            }
            ErrorCode::CursorExpired | ErrorCode::ResyncRequired => StatusCode::CONFLICT,
            ErrorCode::CapacityExceeded => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::ReplicaUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::ReadOnly => StatusCode::METHOD_NOT_ALLOWED,
        }
    }
    fn error_response(&self) -> HttpResponse {
        let mut response = HttpResponse::build(self.status_code());
        match self.code {
            ErrorCode::CapacityExceeded => {
                response.insert_header(("Retry-After", "1"));
            }
            ErrorCode::ReadOnly => {
                response.insert_header(("Allow", "GET, HEAD"));
            }
            _ => {}
        }
        response.json(ErrorResponse {
            error: self.clone(),
        })
    }
}

impl From<ApiError> for tonic::Status {
    fn from(error: ApiError) -> Self {
        use tonic::Code;
        let code = match error.code {
            ErrorCode::InvalidQuery | ErrorCode::InvalidCursor | ErrorCode::CursorMismatch => {
                Code::InvalidArgument
            }
            ErrorCode::CursorExpired | ErrorCode::ResyncRequired => Code::FailedPrecondition,
            ErrorCode::CapacityExceeded => Code::ResourceExhausted,
            ErrorCode::ReplicaUnavailable => Code::Unavailable,
            ErrorCode::NotFound => Code::NotFound,
            ErrorCode::ReadOnly => Code::PermissionDenied,
        };
        Self::with_details(
            code,
            error.message.clone(),
            serde_json::to_vec(&ErrorResponse { error })
                .expect("error JSON")
                .into(),
        )
    }
}
impl From<tonic::Status> for ApiError {
    fn from(error: tonic::Status) -> Self {
        Self::new(
            if matches!(
                error.code(),
                tonic::Code::FailedPrecondition | tonic::Code::OutOfRange
            ) {
                ErrorCode::ResyncRequired
            } else {
                ErrorCode::ReplicaUnavailable
            },
            error.message(),
        )
    }
}
