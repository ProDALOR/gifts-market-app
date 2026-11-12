//! One transport/enrollment implementation for API replicas and independent consumers.
use crate::registration::{self, NodeCredentials};
use std::{path::PathBuf, time::Duration};
use tonic::{
    Status,
    metadata::{Ascii, MetadataValue},
    transport::{Certificate, Channel, ClientTlsConfig, Identity},
};
#[derive(Clone)]
pub struct NodeConfig {
    pub endpoint: String,
    pub identity: NodeCredentials,
    pub registration_token: Option<String>,
    pub ca: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
}
impl NodeConfig {
    pub fn new(endpoint: String, identity: NodeCredentials) -> Self {
        Self {
            endpoint,
            identity,
            registration_token: None,
            ca: None,
            client_cert: None,
            client_key: None,
        }
    }
    pub fn validate(&self) -> Result<(), Status> {
        let uri: tonic::codegen::http::Uri = self
            .endpoint
            .parse()
            .map_err(|_| Status::invalid_argument("invalid core URI"))?;
        match uri.scheme_str() {
            Some("https") => {
                if self.ca.is_none() {
                    return Err(Status::invalid_argument(
                        "HTTPS requires a trusted core CA file",
                    ));
                }
            }
            Some("http")
                if matches!(
                    uri.host(),
                    Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
                ) => {}
            _ => return Err(Status::invalid_argument("use HTTPS outside loopback")),
        }
        if self.client_cert.is_some() != self.client_key.is_some() {
            return Err(Status::invalid_argument(
                "client certificate and key must be supplied together",
            ));
        }
        self.identity.validate()?;
        if let Some(token) = &self.registration_token {
            registration::validate_token(token)?;
        }
        Ok(())
    }
    pub fn authorization(&self) -> Result<MetadataValue<Ascii>, Status> {
        registration::authorization(&self.identity.token)
    }
    pub async fn connect(&self) -> Result<Channel, Status> {
        self.connect_with_rpc_timeout(Duration::from_secs(5)).await
    }
    /// Catalog compilation and durable installation may need a longer unary deadline.
    pub async fn connect_with_rpc_timeout(&self, timeout: Duration) -> Result<Channel, Status> {
        self.validate()?;
        if timeout.is_zero() {
            return Err(Status::invalid_argument("RPC timeout must be positive"));
        }
        let channel = self.channel(timeout).await?;
        registration::register(
            channel.clone(),
            &self.identity,
            self.registration_token.as_deref(),
        )
        .await?;
        Ok(channel)
    }
    async fn channel(&self, timeout: Duration) -> Result<Channel, Status> {
        let mut endpoint = Channel::from_shared(self.endpoint.clone())
            .map_err(|_| Status::invalid_argument("invalid core URI"))?
            .connect_timeout(Duration::from_secs(5))
            .timeout(timeout);
        if let Some(ca) = &self.ca {
            let pem = tokio::fs::read(ca)
                .await
                .map_err(|_| Status::invalid_argument("cannot read core CA"))?;
            let mut tls = ClientTlsConfig::new().ca_certificate(Certificate::from_pem(pem));
            if let (Some(cert), Some(key)) = (&self.client_cert, &self.client_key) {
                tls = tls.identity(Identity::from_pem(
                    tokio::fs::read(cert)
                        .await
                        .map_err(|_| Status::invalid_argument("cannot read client certificate"))?,
                    tokio::fs::read(key)
                        .await
                        .map_err(|_| Status::invalid_argument("cannot read client key"))?,
                ));
            }
            endpoint = endpoint
                .tls_config(tls)
                .map_err(|_| Status::invalid_argument("invalid TLS configuration"))?;
        }
        endpoint
            .connect()
            .await
            .map_err(|_| Status::unavailable("core connection failed"))
    }
}
