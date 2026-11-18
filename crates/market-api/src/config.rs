//! Service limits, validated before the listener is opened.
use std::{net::SocketAddr, path::PathBuf, time::Duration};
#[derive(Clone, Debug)]
pub struct WebSocketLimits {
    pub connections: usize,
    pub subscriptions: usize,
    pub requests: usize,
    pub frame_bytes: usize,
    pub window_bytes: usize,
    pub queue_messages: usize,
    pub queue_bytes: usize,
    pub setup_timeout: Duration,
    pub request_timeout: Duration,
    pub write_timeout: Duration,
    pub heartbeat: Duration,
    pub idle_timeout: Duration,
}
impl Default for WebSocketLimits {
    fn default() -> Self {
        Self {
            connections: 256,
            subscriptions: 16,
            requests: 8,
            frame_bytes: 64 * 1024,
            window_bytes: 4 * 1024 * 1024,
            queue_messages: 8,
            queue_bytes: 8 * 1024 * 1024,
            setup_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(5),
            heartbeat: Duration::from_secs(15),
            idle_timeout: Duration::from_secs(35),
        }
    }
}
#[derive(Clone, Debug)]
pub struct ApiLimits {
    pub json_bytes: usize,
    pub http_connections: usize,
    pub request_timeout: Duration,
    pub websocket: WebSocketLimits,
}
impl Default for ApiLimits {
    fn default() -> Self {
        Self {
            json_bytes: 64 * 1024,
            http_connections: 2048,
            request_timeout: Duration::from_secs(10),
            websocket: WebSocketLimits::default(),
        }
    }
}
impl ApiLimits {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        let ws = &self.websocket;
        anyhow::ensure!(
            self.json_bytes > 0
                && self.http_connections > 0
                && ws.connections > 0
                && ws.subscriptions > 0
                && ws.requests > 0
                && ws.frame_bytes > 0
                && ws.window_bytes > 0
                && ws.queue_messages > 0,
            "API limits must be positive"
        );
        anyhow::ensure!(
            ws.queue_bytes >= ws.window_bytes.saturating_add(128)
                && ws.queue_bytes <= u32::MAX as usize,
            "WebSocket byte budget must hold a page envelope and fit in u32"
        );
        anyhow::ensure!(
            !self.request_timeout.is_zero()
                && !ws.setup_timeout.is_zero()
                && !ws.request_timeout.is_zero()
                && !ws.write_timeout.is_zero()
                && !ws.heartbeat.is_zero()
                && ws.idle_timeout > ws.heartbeat,
            "API deadlines must be positive and idle timeout must exceed heartbeat"
        );
        Ok(())
    }
}
pub struct ApiConfig {
    pub bind: SocketAddr,
    pub workers: usize,
    pub web_dir: Option<PathBuf>,
    pub cors_origins: Vec<String>,
    pub limits: ApiLimits,
}
impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".parse().unwrap(),
            workers: 2,
            web_dir: None,
            cors_origins: vec![],
            limits: ApiLimits::default(),
        }
    }
}
