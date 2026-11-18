//! Browser security headers shared by all API and frontend routes.
use actix_web::middleware::DefaultHeaders;
#[cfg(feature = "miniapp")]
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self' https://telegram.org/js/telegram-web-app.js; connect-src 'self'; style-src 'self'; frame-ancestors 'self' https://web.telegram.org https://*.web.telegram.org https://webk.telegram.org https://webz.telegram.org; object-src 'none'; base-uri 'self'";
#[cfg(not(feature = "miniapp"))]
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; connect-src 'self'; style-src 'self'; frame-ancestors 'self'; object-src 'none'; base-uri 'self'";
const HEADERS: [(&str, &str); 4] = [
    ("x-content-type-options", "nosniff"),
    ("referrer-policy", "no-referrer"),
    ("cache-control", "no-store"),
    ("content-security-policy", CONTENT_SECURITY_POLICY),
];
pub fn security_headers() -> DefaultHeaders {
    HEADERS
        .into_iter()
        .fold(DefaultHeaders::new(), |headers, pair| headers.add(pair))
}
/// Framework errors bypass DefaultHeaders' successful response path.
pub(crate) fn apply_headers(mut response: actix_web::HttpResponse) -> actix_web::HttpResponse {
    use actix_web::http::header::{HeaderName, HeaderValue};
    for (name, value) in HEADERS {
        let name = HeaderName::from_static(name);
        if !response.headers().contains_key(&name) {
            response
                .headers_mut()
                .insert(name, HeaderValue::from_static(value));
        }
    }
    response
}
