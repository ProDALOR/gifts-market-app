//! Command-line configuration owned by each independently deployed API process.
use crate::replication::ReplicaConfig;
use clap::Parser;
use market_reader::registration::NodeCredentials;
use market_view::protocol::wire::NodeRole;
use std::{net::SocketAddr, path::PathBuf, time::Duration};

fn positive_duration(value: &str) -> Result<Duration, String> {
    let duration = humantime::parse_duration(value).map_err(|error| error.to_string())?;
    if duration.is_zero() {
        return Err("duration must be positive".into());
    }
    Ok(duration)
}

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
    about = "Public market API backed by its own synchronized state"
)]
pub struct Cli {
    /// Public HTTP listen address for this API process.
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub bind: SocketAddr,
    /// Optional read-only gRPC listener for downstream services.
    #[arg(long)]
    pub grpc_bind: Option<SocketAddr>,
    #[arg(long, requires = "grpc_key")]
    pub grpc_cert: Option<PathBuf>,
    #[arg(long, requires = "grpc_cert")]
    pub grpc_key: Option<PathBuf>,
    /// Require client certificates signed by this CA on the gRPC listener.
    #[arg(long, requires = "grpc_cert")]
    pub grpc_client_ca: Option<PathBuf>,
    /// Optional downstream gRPC key; unrelated to the core node identity.
    #[arg(long, env = "MARKET_API_TOKEN", hide_env_values = true)]
    pub grpc_token: Option<String>,
    /// Browser origins allowed in addition to the API's own origin. Repeat for each GUI.
    #[arg(long)]
    pub cors_origin: Vec<String>,
    #[arg(long, default_value_t = 128)]
    pub max_streams: usize,
    #[arg(long, default_value_t = 64)]
    pub max_snapshots: usize,
    #[arg(long, default_value_t = 536870912)]
    pub max_snapshot_bytes: usize,
    #[arg(long, default_value="60s", value_parser=positive_duration)]
    pub snapshot_ttl: Duration,
    #[arg(long, default_value_t = 4096)]
    pub replay_batches: usize,
    #[arg(long, default_value_t = 33554432)]
    pub replay_bytes: usize,
    /// Private core read listener shared by all API processes.
    #[arg(
        long,
        env = "MARKET_CORE_URL",
        default_value = "http://127.0.0.1:50052"
    )]
    pub core: String,
    /// Stable node ID; generated and saved on first start when omitted.
    #[arg(long, env = "MARKET_NODE_ID")]
    pub node_id: Option<String>,
    /// Private credential file; each API process needs its own persistent file.
    #[arg(
        long,
        env = "MARKET_NODE_CREDENTIALS",
        default_value = ".market/market-app.json"
    )]
    pub credentials: PathBuf,
    /// Only needed for first enrollment, or after the core registry is removed.
    #[arg(long, env = "MARKET_REGISTRATION_TOKEN", hide_env_values = true)]
    pub registration_token: Option<String>,
    #[arg(long)]
    pub core_ca: Option<PathBuf>,
    #[arg(long)]
    pub client_cert: Option<PathBuf>,
    #[arg(long)]
    pub client_key: Option<PathBuf>,
    #[arg(long, default_value = "5m", value_parser = positive_duration)]
    pub max_stale: Duration,
    #[arg(long, default_value_t = 32)]
    pub max_concurrent_queries: usize,
    #[arg(long, default_value_t = 5000)]
    pub max_scan: usize,
    #[arg(long, default_value_t = 2)]
    pub workers: usize,
}

impl Cli {
    pub fn replica_config(&self) -> anyhow::Result<ReplicaConfig> {
        anyhow::ensure!(
            self.max_concurrent_queries > 0
                && self.max_scan > 0
                && self.max_scan <= 100_000
                && self.workers > 0
                && self.max_streams > 0
                && self.max_snapshots > 0
                && self.max_snapshot_bytes > 0
                && self.max_snapshot_bytes <= u32::MAX as usize
                && self.replay_batches > 0
                && self.replay_bytes > 0,
            "invalid API limits"
        );
        anyhow::ensure!(
            self.grpc_bind.is_none_or(|addr| addr.ip().is_loopback()) || self.grpc_cert.is_some(),
            "non-loopback gRPC listeners require TLS"
        );
        anyhow::ensure!(
            self.grpc_token.as_ref().is_none_or(|v| v.len() >= 16
                && v.len() <= 1024
                && v.bytes().all(|b| b.is_ascii_graphic())),
            "API token must contain 16..1024 visible ASCII bytes"
        );
        anyhow::ensure!(
            self.cors_origin.iter().all(|v| valid_origin(v)),
            "CORS origins must be exact http(s) origins without a trailing slash"
        );
        let identity = NodeCredentials::load_or_create(
            &self.credentials,
            self.node_id.as_deref(),
            NodeRole::Reader,
        )?;
        let mut config = ReplicaConfig::new(self.core.clone(), identity);
        config.node.registration_token = self.registration_token.clone();
        config.node.ca = self.core_ca.clone();
        config.node.client_cert = self.client_cert.clone();
        config.node.client_key = self.client_key.clone();
        config.validate()?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_origins_are_exact_and_listener_tls_options_are_paired() {
        for origin in ["http://localhost:3000", "https://gui.example"] {
            assert!(valid_origin(origin));
        }
        for origin in [
            "*",
            "null",
            "https://gui.example/",
            "https://gui.example/path",
            "https://gui.example?q=a",
            "https://user@gui.example",
            "file:///tmp",
        ] {
            assert!(!valid_origin(origin));
        }
        assert!(Cli::try_parse_from(["market-app", "--grpc-cert", "cert.pem"]).is_err());
        assert!(Cli::try_parse_from(["market-app", "--grpc-client-ca", "ca.pem"]).is_err());
    }
    #[test]
    fn stale_window_uses_explicit_duration_units_without_a_seconds_alias() {
        let args = |value| Cli::try_parse_from(["market-app", "--max-stale", value]);
        assert_eq!(args("2m 30s").unwrap().max_stale, Duration::from_secs(150));
        assert_eq!(args("250ms").unwrap().max_stale, Duration::from_millis(250));
        for invalid in ["0s", "300", "-1s", "invalid"] {
            assert!(args(invalid).is_err());
        }
        assert!(Cli::try_parse_from(["market-app", "--max-stale-seconds", "300"]).is_err());
    }
}
