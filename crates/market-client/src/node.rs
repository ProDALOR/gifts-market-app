//! One configured node and one reusable HTTP/2 channel. Configure before first use.
use crate::registration::{self, NodeCredentials};
use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::OnceCell;
use tonic::{
    Code, Status,
    metadata::{Ascii, MetadataValue},
    transport::{Certificate, Channel, ClientTlsConfig, Identity},
};
#[derive(Clone)]
pub struct NodeConfig {
    pub endpoint: String,
    pub identity: NodeCredentials,
    pub join_token: Option<String>,
    pub ca: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    channel: Arc<OnceCell<Channel>>,
}
impl NodeConfig {
    pub fn new(endpoint: String, identity: NodeCredentials) -> Self {
        Self {
            endpoint,
            identity,
            join_token: None,
            ca: None,
            client_cert: None,
            client_key: None,
            channel: Arc::new(OnceCell::new()),
        }
    }
    pub fn validate(&self) -> Result<(), Status> {
        self.identity.validate()?;
        let uri: tonic::codegen::http::Uri = self
            .endpoint
            .parse()
            .map_err(|_| Status::invalid_argument("invalid core URI"))?;
        if uri.authority().is_none_or(|a| a.as_str().contains('@'))
            || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
        {
            return Err(Status::invalid_argument(
                "core URI must contain only scheme and authority",
            ));
        }
        match uri.scheme_str() {
            Some("https") => {}
            Some("http")
                if uri.host().is_some_and(|host| {
                    host == "localhost"
                        || host
                            .trim_matches(['[', ']'])
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                }) =>
            {
                if self.ca.is_some() || self.client_cert.is_some() || self.client_key.is_some() {
                    return Err(Status::invalid_argument(
                        "TLS options require an HTTPS core URI",
                    ));
                }
            }
            _ => return Err(Status::invalid_argument("use HTTPS outside loopback")),
        }
        if self.client_cert.is_some() != self.client_key.is_some() {
            return Err(Status::invalid_argument(
                "client certificate and key must be supplied together",
            ));
        }
        if let Some(token) = &self.join_token {
            registration::validate_token(token)?;
            if token == &self.identity.token {
                return Err(Status::invalid_argument("join and node tokens must differ"));
            }
        }
        Ok(())
    }
    pub fn authorization(&self) -> Result<MetadataValue<Ascii>, Status> {
        registration::authorization(&self.identity.token)
    }
    /// Clones share the channel. Tonic reconnects the underlying transport after failure.
    /// Each caller sets its own deadline; no channel-wide timeout can truncate streams
    /// or slow catalog commits. Connecting never performs enrollment.
    pub async fn connect(&self) -> Result<Channel, Status> {
        self.channel
            .get_or_try_init(|| async {
                self.validate()?;
                let mut endpoint = Channel::from_shared(self.endpoint.clone())
                    .map_err(|_| Status::invalid_argument("invalid core URI"))?
                    .connect_timeout(Duration::from_secs(5));
                if self.endpoint.starts_with("https://") {
                    let mut tls = if let Some(path) = &self.ca {
                        ClientTlsConfig::new()
                            .ca_certificate(Certificate::from_pem(read(path).await?))
                    } else {
                        ClientTlsConfig::new().with_native_roots()
                    };
                    if let (Some(cert), Some(key)) = (&self.client_cert, &self.client_key) {
                        tls = tls.identity(Identity::from_pem(read(cert).await?, read(key).await?));
                    }
                    endpoint = endpoint
                        .tls_config(tls)
                        .map_err(|_| Status::invalid_argument("invalid TLS configuration"))?;
                }
                Ok(endpoint.connect_lazy())
            })
            .await
            .cloned()
    }
    /// Try the real operation first. Only an unknown credential triggers enrollment;
    /// permission, network and deadline failures never spend the join credential.
    /// Retry once with the same node identity, including after a lost enrollment reply.
    pub async fn authenticated<T, F, Fut>(&self, mut operation: F) -> Result<T, Status>
    where
        F: FnMut(Channel, MetadataValue<Ascii>) -> Fut,
        Fut: Future<Output = Result<T, Status>>,
    {
        let channel = self.connect().await?;
        let auth = self.authorization()?;
        match operation(channel.clone(), auth.clone()).await {
            Err(error) if error.code() == Code::Unauthenticated => {
                let token = self.join_token.as_deref().ok_or_else(|| {
                    Status::unauthenticated(
                        "node is not registered; supply MARKET_JOIN_TOKEN for enrollment",
                    )
                })?;
                registration::register(channel.clone(), &self.identity, token).await?;
                operation(channel, auth).await
            }
            result => result,
        }
    }
}
async fn read(path: &std::path::Path) -> Result<Vec<u8>, Status> {
    tokio::fs::read(path)
        .await
        .map_err(|_| Status::invalid_argument("cannot read TLS material"))
}
