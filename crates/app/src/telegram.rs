use crate::error::{ApiError, ErrorCode, ErrorResponse, Result};
use actix_web::{HttpRequest, middleware::DefaultHeaders, web};
use market_telegram_app::{
    auth::{Auth, Identity, Session},
    bot::app_url,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;
use utoipa::ToSchema;
#[derive(Clone)]
pub struct Telegram {
    auth: Option<Auth>,
    username: Option<String>,
    public_url: Option<String>,
    slots: Arc<Semaphore>,
}
impl Telegram {
    pub fn new(
        auth: Option<Auth>,
        username: Option<String>,
        public_url: Option<String>,
    ) -> anyhow::Result<Self> {
        let username = username
            .filter(|v| !v.is_empty())
            .map(|v| v.strip_prefix('@').unwrap_or(&v).to_owned());
        anyhow::ensure!(
            username.as_ref().is_none_or(|v| (5..=32).contains(&v.len())
                && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')),
            "Invalid Telegram bot username"
        );
        let public_url = public_url
            .filter(|v| !v.is_empty())
            .map(|v| app_url(&v).map(String::from))
            .transpose()?;
        Ok(Self {
            auth,
            username,
            public_url,
            slots: Arc::new(Semaphore::new(16)),
        })
    }
}
#[derive(Debug, Serialize, ToSchema)]
pub struct TelegramConfig {
    pub bot_id: Option<String>,
    pub bot_username: Option<String>,
    pub public_url: Option<String>,
    pub auth_enabled: bool,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SignIn {
    pub init_data: String,
}
fn unavailable() -> ApiError {
    ApiError {
        code: ErrorCode::Unavailable,
        message: "Telegram sign-in is not configured".into(),
    }
}
#[utoipa::path(get,path="/telegram",responses((status=200,body=TelegramConfig)))]
pub async fn config(state: Option<web::Data<Telegram>>) -> web::Json<TelegramConfig> {
    web::Json(TelegramConfig {
        bot_id: state
            .as_ref()
            .and_then(|s| s.auth.as_ref())
            .map(|a| a.bot_id().to_string()),
        bot_username: state.as_ref().and_then(|s| s.username.clone()),
        public_url: state.as_ref().and_then(|s| s.public_url.clone()),
        auth_enabled: state.as_ref().is_some_and(|s| s.auth.is_some()),
    })
}
#[utoipa::path(post,path="/auth/telegram",request_body=SignIn,responses((status=200,body=Session),(status=401,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub async fn sign_in(
    state: Option<web::Data<Telegram>>,
    input: web::Json<SignIn>,
) -> Result<web::Json<Session>> {
    let state = state.ok_or_else(unavailable)?;
    let auth = state.auth.clone().ok_or_else(unavailable)?;
    if input.init_data.len() > 16 * 1024 {
        return Err(ApiError::invalid("Telegram sign-in data is too large"));
    }
    let permit = state
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError {
            code: ErrorCode::CapacityExceeded,
            message: "Sign-in is busy. Please try again".into(),
        })?;
    let input = input.into_inner();
    let session = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        auth.sign_in(&input.init_data, jiff::Timestamp::now())
    })
    .await
    .map_err(|_| unavailable())??;
    Ok(web::Json(session))
}
#[utoipa::path(get,path="/auth/me",responses((status=200,body=Identity),(status=401,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub async fn me(
    state: Option<web::Data<Telegram>>,
    request: HttpRequest,
) -> Result<web::Json<Identity>> {
    let state = state.ok_or_else(unavailable)?;
    let auth = state.auth.as_ref().ok_or_else(unavailable)?;
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(market_telegram_app::auth::AuthError::Invalid)?;
    Ok(web::Json(auth.identity(token, jiff::Timestamp::now())?))
}
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/telegram", web::get().to(config))
        .route("/auth/telegram", web::post().to(sign_in))
        .route("/auth/me", web::get().to(me));
}
pub fn security_headers() -> DefaultHeaders {
    DefaultHeaders::new()
        .add(("X-Content-Type-Options","nosniff"))
        .add(("Referrer-Policy","no-referrer"))
        .add(("Cache-Control","no-store"))
        .add(("Content-Security-Policy","default-src 'self'; script-src 'self' https://telegram.org/js/telegram-web-app.js; connect-src 'self'; style-src 'self'; frame-ancestors 'self' https://web.telegram.org https://*.web.telegram.org https://webk.telegram.org https://webz.telegram.org; object-src 'none'; base-uri 'self'"))
}
