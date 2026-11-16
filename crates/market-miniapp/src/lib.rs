//! Telegram Mini App authentication and public launch configuration, independent of bot transport.
pub mod auth;
use url::Url;

pub fn app_url(raw: &str) -> anyhow::Result<Url> {
    let url = Url::parse(raw).map_err(|_| anyhow::anyhow!("Invalid public app URL"))?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "Public app URL must be an HTTPS origin without credentials, query or fragment"
    );
    Ok(url)
}
