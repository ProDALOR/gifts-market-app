//! Browser security headers shared by all API and frontend routes.
use actix_web::middleware::DefaultHeaders;
#[cfg(feature = "miniapp")]
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self' https://telegram.org/js/telegram-web-app.js; connect-src 'self'; style-src 'self'; frame-ancestors 'self' https://web.telegram.org https://*.web.telegram.org https://webk.telegram.org https://webz.telegram.org; object-src 'none'; base-uri 'self'";
#[cfg(not(feature = "miniapp"))]
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; connect-src 'self'; style-src 'self'; frame-ancestors 'self'; object-src 'none'; base-uri 'self'";
pub fn security_headers() -> DefaultHeaders {
    DefaultHeaders::new()
        .add(("X-Content-Type-Options", "nosniff"))
        .add(("Referrer-Policy", "no-referrer"))
        .add(("Cache-Control", "no-store"))
        .add(("Content-Security-Policy", CONTENT_SECURITY_POLICY))
}
