use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};
#[derive(Parser)]
#[command(version, about = "Public market GUI and read-only API")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    #[arg(long, env = "MARKET_HTTP_ADDR", default_value = "127.0.0.1:8080")]
    pub bind: SocketAddr,
    #[arg(
        long,
        env = "MARKET_CORE_URL",
        default_value = "http://127.0.0.1:50051"
    )]
    pub core: String,
    #[arg(long, env = "MARKET_NODE_TOKEN", hide_env_values = true)]
    pub node_token: Option<String>,
    #[arg(long, env = "MARKET_DATA_DIR", default_value = "data")]
    pub data_dir: PathBuf,
    #[arg(long, default_value = "web/dist")]
    pub web_dir: PathBuf,
    #[arg(long)]
    pub cors_origin: Vec<String>,
    #[arg(long, default_value_t = 32)]
    pub max_concurrent_queries: usize,
    #[arg(long, default_value_t = 128)]
    pub max_streams: usize,
    #[arg(long, default_value_t = 2)]
    pub workers: usize,
}
#[derive(Subcommand)]
pub enum Command {
    Schema,
}
