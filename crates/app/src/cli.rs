//! Configuration for a stateless REST/WebSocket/gRPC gateway to Market Hub.
use clap::Parser;
use market_client::{node::NodeConfig, registration::NodeCredentials};
use std::{net::SocketAddr, path::PathBuf};
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
    pub grpc_bind: Option<SocketAddr>,
    #[arg(long, requires = "grpc_key")]
    pub grpc_cert: Option<PathBuf>,
    #[arg(long, requires = "grpc_cert")]
    pub grpc_key: Option<PathBuf>,
    #[arg(long, requires = "grpc_cert")]
    pub grpc_client_ca: Option<PathBuf>,
    #[arg(long, env = "MARKET_API_TOKEN", hide_env_values = true)]
    pub grpc_token: Option<String>,
    #[arg(long)]
    pub cors_origin: Vec<String>,
    #[arg(
        long,
        env = "MARKET_CORE_URL",
        default_value = "http://127.0.0.1:50051"
    )]
    pub core: String,
    #[arg(long, env = "MARKET_NODE_ID")]
    pub node_id: Option<String>,
    #[arg(
        long,
        env = "MARKET_NODE_CREDENTIALS",
        default_value = ".market/market-app.json"
    )]
    pub credentials: PathBuf,
    #[arg(long, env = "MARKET_JOIN_TOKEN", hide_env_values = true)]
    pub join_token: Option<String>,
    #[arg(long)]
    pub core_ca: Option<PathBuf>,
    #[arg(long, requires = "client_key")]
    pub client_cert: Option<PathBuf>,
    #[arg(long, requires = "client_cert")]
    pub client_key: Option<PathBuf>,
    #[arg(long, default_value_t = 32)]
    pub max_concurrent_queries: usize,
    #[arg(long, default_value_t = 128)]
    pub max_streams: usize,
    #[arg(long, default_value_t = 2)]
    pub workers: usize,
}
impl Cli {
    pub fn node_config(&self) -> anyhow::Result<NodeConfig> {
        anyhow::ensure!(
            self.max_concurrent_queries > 0 && self.max_streams > 0 && self.workers > 0,
            "gateway limits must be positive"
        );
        anyhow::ensure!(
            self.grpc_bind.is_none_or(|a| a.ip().is_loopback()) || self.grpc_cert.is_some(),
            "remote downstream gRPC requires TLS"
        );
        anyhow::ensure!(
            self.cors_origin.iter().all(|v| valid_origin(v)),
            "CORS origins must be exact HTTP(S) origins"
        );
        anyhow::ensure!(
            self.grpc_token.as_ref().is_none_or(
                |v| (16..=512).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_graphic())
            ),
            "invalid downstream API credential"
        );
        let identity = NodeCredentials::load_or_create(&self.credentials, self.node_id.as_deref())?;
        let mut node = NodeConfig::new(self.core.clone(), identity);
        node.join_token = self.join_token.clone().filter(|t| !t.is_empty());
        node.ca = self.core_ca.clone();
        node.client_cert = self.client_cert.clone();
        node.client_key = self.client_key.clone();
        node.validate()?;
        Ok(node)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_origins_and_paired_tls_arguments() {
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
