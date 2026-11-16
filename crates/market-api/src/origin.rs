//! One canonical origin policy for CORS and WebSocket handshakes.
use actix_web::{HttpRequest, http::header};
use std::collections::BTreeSet;
use url::Url;
#[derive(Clone, Default)]
pub(crate) struct OriginPolicy {
    allowed: BTreeSet<String>,
}
fn canonical(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| url.origin().ascii_serialization())
}
impl OriginPolicy {
    pub fn new(origins: &[String]) -> anyhow::Result<Self> {
        let mut allowed = BTreeSet::new();
        for raw in origins {
            allowed.insert(canonical(raw).ok_or_else(||anyhow::anyhow!("CORS origins must be HTTP(S) origins without credentials, paths, queries or fragments"))?);
        }
        Ok(Self { allowed })
    }
    pub fn allows(&self, origin: &str, host: Option<&str>) -> bool {
        let Some(origin) = canonical(origin) else {
            return false;
        };
        self.allowed.contains(&origin)
            || host.is_some_and(|host| {
                ["http", "https"].iter().any(|scheme| {
                    canonical(&format!("{scheme}://{host}")).as_ref() == Some(&origin)
                })
            })
    }
    pub fn check(&self, request: &HttpRequest) -> crate::error::Result<()> {
        if let Some(origin) = request.headers().get(header::ORIGIN) {
            let valid = origin.to_str().ok().is_some_and(|origin| {
                self.allows(
                    origin,
                    request
                        .headers()
                        .get(header::HOST)
                        .and_then(|value| value.to_str().ok()),
                )
            });
            if !valid {
                return Err(crate::error::ApiError::forbidden("Origin is not allowed"));
            }
        }
        Ok(())
    }
    pub fn cors(&self) -> actix_cors::Cors {
        let policy = self.clone();
        actix_cors::Cors::default()
            .allowed_methods(vec!["GET", "POST", "HEAD"])
            .allowed_headers(["Content-Type", "Authorization"])
            .allowed_origin_fn(move |origin, head| {
                origin.to_str().ok().is_some_and(|origin| {
                    policy.allows(
                        origin,
                        head.headers
                            .get(header::HOST)
                            .and_then(|value| value.to_str().ok()),
                    )
                })
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cors_and_websockets_use_canonical_validated_origins() {
        let policy = OriginPolicy::new(&["https://EXAMPLE.com:443/".into()]).unwrap();
        assert!(policy.allows("https://example.com", None));
        assert!(policy.allows("http://localhost:8080", Some("localhost:8080")));
        assert!(!policy.allows("http://localhost:8081", Some("localhost:8080")));
        for origin in [
            "null",
            "https://example.com/path",
            "https://user:pass@example.com",
            "https://example.com?x=1",
            "*",
        ] {
            assert!(OriginPolicy::new(&[origin.into()]).is_err());
            assert!(!policy.allows(origin, None));
        }
    }
}
