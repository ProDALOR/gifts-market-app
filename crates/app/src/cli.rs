//! Configuration for a stateless REST/WebSocket gateway to Market Hub.
use crate::client::CoreClient;
use clap::Parser;
use std::net::SocketAddr;
fn valid_origin(value: &str) -> bool {
    let Ok(uri) = value.parse::<actix_web::http::Uri>() else {
        return false;
    };
    matches!(uri.scheme_str(), Some("http" | "https"))
        && uri.authority().is_some_and(|v| !v.as_str().contains('@'))
        && !value.ends_with('/')
        && uri.path_and_query().is_none_or(|v| v.as_str() == "/")
}
#[derive(Parser)]
#[command(
    version,
    about = "Stateless market gateway; core owns market queries and history"
)]
pub struct Cli {
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub bind: SocketAddr,
    #[arg(long)]
    pub cors_origin: Vec<String>,
    #[arg(
        long,
        env = "MARKET_CORE_URL",
        default_value = "http://127.0.0.1:50051"
    )]
    pub core: String,
    #[arg(long, env = "MARKET_NODE_TOKEN", hide_env_values = true)]
    pub node_token: String,
    #[arg(long, default_value_t = 32)]
    pub max_concurrent_queries: usize,
    #[arg(long, default_value_t = 128)]
    pub max_streams: usize,
    #[arg(long, default_value_t = 2)]
    pub workers: usize,
}
impl Cli {
    pub fn core_client(&self) -> anyhow::Result<CoreClient> {
        anyhow::ensure!(
            self.max_concurrent_queries > 0 && self.max_streams > 0 && self.workers > 0,
            "gateway limits must be positive"
        );
        anyhow::ensure!(
            self.cors_origin.iter().all(|v| valid_origin(v)),
            "CORS origins must be exact HTTP(S) origins"
        );
        let node = CoreClient::new(self.core.clone(), self.node_token.clone())?;
        Ok(node)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_origins_and_removed_certificate_arguments() {
        assert!(valid_origin("https://gui.example"));
        for value in [
            "*",
            "https://gui.example/",
            "https://user@gui.example",
            "https://gui.example/path",
        ] {
            assert!(!valid_origin(value));
        }
        assert!(Cli::try_parse_from(["market-app", "--grpc-cert", "cert.pem"]).is_err());
        assert!(Cli::try_parse_from(["market-app", "--max-stale", "5m"]).is_err());
    }
}
