//! HTTP adapters for Mini App sign-in and launch settings; bot application logic lives in market-telegram-bot.
use crate::error::{ApiError, ErrorCode, ErrorResponse, Result};
use actix_web::{HttpRequest, web};
use market_miniapp::{
    app_url,
    auth::{Auth, Identity, Session},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use tokio::sync::Semaphore;
use utoipa::{OpenApi, ToSchema};
#[derive(OpenApi)]
#[openapi(paths(config, sign_in, me))]
pub(crate) struct MiniAppDoc;
#[derive(Clone)]
pub struct MiniAppApi {
    identity: Arc<RwLock<BotIdentity>>,
    public_url: Option<String>,
    slots: Arc<Semaphore>,
}
#[derive(Clone)]
struct BotIdentity {
    auth: Option<Auth>,
    username: Option<String>,
}
impl MiniAppApi {
    pub fn new(
        auth: Option<Auth>,
        username: Option<String>,
        public_url: Option<String>,
    ) -> anyhow::Result<Self> {
        let identity = Self::bot_identity(auth, username)?;
        let public_url = public_url
            .filter(|v| !v.is_empty())
            .map(|v| app_url(&v).map(String::from))
            .transpose()?;
        Ok(Self {
            identity: Arc::new(RwLock::new(identity)),
            public_url,
            slots: Arc::new(Semaphore::new(16)),
        })
    }
    fn bot_identity(auth: Option<Auth>, username: Option<String>) -> anyhow::Result<BotIdentity> {
        let username = username
            .filter(|v| !v.is_empty())
            .map(|v| v.strip_prefix('@').unwrap_or(&v).to_owned());
        anyhow::ensure!(
            username.as_ref().is_none_or(|v| (5..=32).contains(&v.len())
                && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')),
            "Invalid Telegram bot username"
        );
        Ok(BotIdentity { auth, username })
    }
    /// Atomically publish authenticated bot identity to all HTTP workers.
    /// Existing identity remains usable during transport outages.
    pub fn set_identity(&self, auth: Auth, username: Option<String>) -> anyhow::Result<()> {
        let identity = Self::bot_identity(Some(auth), username)?;
        *self
            .identity
            .write()
            .map_err(|_| anyhow::anyhow!("Mini App identity is unavailable"))? = identity;
        Ok(())
    }
    fn identity(&self) -> Result<BotIdentity> {
        self.identity
            .read()
            .map(|identity| identity.clone())
            .map_err(|_| unavailable())
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
#[utoipa::path(get,path="/telegram",tag="telegram",responses((status=200,body=TelegramConfig)))]
pub async fn config(state: Option<web::Data<MiniAppApi>>) -> Result<web::Json<TelegramConfig>> {
    let identity = state.as_ref().map(|s| s.identity()).transpose()?;
    Ok(web::Json(TelegramConfig {
        bot_id: identity
            .as_ref()
            .and_then(|s| s.auth.as_ref())
            .map(|a| a.bot_id().to_string()),
        bot_username: identity.as_ref().and_then(|s| s.username.clone()),
        public_url: state.as_ref().and_then(|s| s.public_url.clone()),
        auth_enabled: identity.as_ref().is_some_and(|s| s.auth.is_some()),
    }))
}
#[utoipa::path(post,path="/auth/telegram",tag="telegram",request_body=SignIn,responses((status=200,body=Session),(status=401,body=ErrorResponse),(status=503,body=ErrorResponse),(status=400,body=ErrorResponse),(status=413,body=ErrorResponse),(status=415,body=ErrorResponse),(status=429,body=ErrorResponse)))]
pub async fn sign_in(
    state: Option<web::Data<MiniAppApi>>,
    input: web::Json<SignIn>,
) -> Result<web::Json<Session>> {
    let state = state.ok_or_else(unavailable)?;
    let auth = state.identity()?.auth.ok_or_else(unavailable)?;
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
#[utoipa::path(get,path="/auth/me",tag="telegram",responses((status=200,body=Identity),(status=401,body=ErrorResponse),(status=503,body=ErrorResponse)))]
pub async fn me(
    state: Option<web::Data<MiniAppApi>>,
    request: HttpRequest,
) -> Result<web::Json<Identity>> {
    let state = state.ok_or_else(unavailable)?;
    let auth = state.identity()?.auth.ok_or_else(unavailable)?;
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(market_miniapp::auth::AuthError::Invalid)?;
    Ok(web::Json(auth.identity(token, jiff::Timestamp::now())?))
}
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/telegram", web::get().to(config))
        .route("/auth/telegram", web::post().to(sign_in))
        .route("/auth/me", web::get().to(me));
}
