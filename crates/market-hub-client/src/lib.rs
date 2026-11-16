//! Authenticated Market Hub transport for full snapshot and Watch replication.
//! Owns credentials, one reusable channel, RPC deadlines and bounded capacity retries.
use market_hub_protocol::wire as w;
use std::time::Duration;
use tonic::{
    Status,
    metadata::{Ascii, MetadataValue},
    transport::Channel,
};
#[derive(Clone)]
pub struct HubClient {
    auth: MetadataValue<Ascii>,
    channel: Channel,
    source: String,
}
impl HubClient {
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
        let source = format!("{endpoint}|{id}");
        let uri: tonic::codegen::http::Uri = endpoint
            .parse()
            .map_err(|_| Status::invalid_argument("invalid hub URI"))?;
        if uri.scheme_str() != Some("http")
            || uri.authority().is_none_or(|a| a.as_str().contains('@'))
            || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
        {
            return Err(Status::invalid_argument(
                "hub URI must be an http:// address without credentials or a path",
            ));
        }
        let mut auth: MetadataValue<Ascii> =
            format!("Bearer {token}").parse().map_err(|_| invalid())?;
        auth.set_sensitive(true);
        let channel = Channel::from_shared(endpoint)
            .map_err(|_| Status::invalid_argument("invalid hub URI"))?
            .connect_timeout(Duration::from_secs(5))
            .connect_lazy();
        Ok(Self {
            auth,
            channel,
            source,
        })
    }
    /// Stable cache binding. Contains the endpoint and service ID, never the token secret.
    pub fn source(&self) -> &str {
        &self.source
    }
    fn request<T>(&self, value: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(value);
        request
            .metadata_mut()
            .insert("authorization", self.auth.clone());
        request
    }
    fn rpc(&self) -> w::market_read_client::MarketReadClient<Channel> {
        w::market_read_client::MarketReadClient::new(self.channel.clone())
            .max_decoding_message_size(1024 * 1024)
    }
    /// One complete, unfiltered snapshot page. Capacity retries reuse the same continuation.
    pub async fn snapshot_page(&self, page_token: String) -> Result<w::Page, Status> {
        for attempt in 0..6 {
            let mut request = self.request(w::ReadRequest {
                limit: 500,
                page_token: page_token.clone(),
                ..Default::default()
            });
            request.set_timeout(Duration::from_secs(10));
            let mut stream = match self.rpc().read(request).await {
                Err(error) if error.code() == tonic::Code::ResourceExhausted && attempt < 5 => {
                    tokio::time::sleep(Duration::from_millis(25 << attempt)).await;
                    continue;
                }
                result => result?.into_inner(),
            };
            let page = stream
                .message()
                .await?
                .ok_or_else(|| Status::data_loss("missing snapshot page"))?;
            if stream.message().await?.is_some() {
                return Err(Status::data_loss("extra snapshot page"));
            }
            return Ok(page);
        }
        unreachable!()
    }
    /// Resume the full replication stream. The timeout covers connection setup only.
    pub async fn watch(&self, after_cursor: String) -> Result<tonic::Streaming<w::Batch>, Status> {
        tokio::time::timeout(
            Duration::from_secs(10),
            self.rpc().watch(self.request(w::WatchRequest {
                after_cursor,
                ..Default::default()
            })),
        )
        .await
        .map_err(|_| Status::deadline_exceeded("sync connection timed out"))?
        .map(|response| response.into_inner())
    }
}

#[cfg(test)]
mod tests;
