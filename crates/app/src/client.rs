//! Validated service identity, cached authorization metadata and one reusable HTTP/2 channel.
use std::{future::Future, time::Duration};
use tonic::{
    Status,
    metadata::{Ascii, MetadataValue},
    transport::Channel,
};
#[derive(Clone)]
pub struct CoreClient {
    id: String,
    auth: MetadataValue<Ascii>,
    channel: Channel,
}
impl CoreClient {
    /// Construct inside the async runtime. No connection or enrollment RPC is required.
    pub fn new(endpoint: String, token: String) -> Result<Self, Status> {
        let invalid =
            || Status::invalid_argument("invalid service token; use a token issued by the hub CLI");
        if !(16..=512).contains(&token.len()) || !token.bytes().all(|b| (33..=126).contains(&b)) {
            return Err(invalid());
        }
        let (id, signature) = token.split_once('.').ok_or_else(invalid)?;
        if !(1..=64).contains(&id.len())
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || signature.is_empty()
        {
            return Err(invalid());
        }
        let uri: tonic::codegen::http::Uri = endpoint
            .parse()
            .map_err(|_| Status::invalid_argument("invalid core URI"))?;
        if uri.scheme_str() != Some("http")
            || uri.authority().is_none_or(|a| a.as_str().contains('@'))
            || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
        {
            return Err(Status::invalid_argument(
                "core URI must be an http:// address without credentials or a path",
            ));
        }
        let mut auth: MetadataValue<Ascii> =
            format!("Bearer {token}").parse().map_err(|_| invalid())?;
        auth.set_sensitive(true);
        let channel = Channel::from_shared(endpoint)
            .map_err(|_| Status::invalid_argument("invalid core URI"))?
            .connect_timeout(Duration::from_secs(5))
            .connect_lazy();
        Ok(Self {
            id: id.into(),
            auth,
            channel,
        })
    }
    pub fn node_id(&self) -> &str {
        &self.id
    }
    pub fn channel(&self) -> Channel {
        self.channel.clone()
    }
    pub fn authorization(&self) -> MetadataValue<Ascii> {
        self.auth.clone()
    }
    /// Each operation supplies its own deadline. Tonic reconnects the shared transport.
    pub async fn authenticated<T, F, Fut>(&self, operation: F) -> Result<T, Status>
    where
        F: FnOnce(Channel, MetadataValue<Ascii>) -> Fut,
        Fut: Future<Output = Result<T, Status>>,
    {
        operation(self.channel(), self.authorization()).await
    }
}
