//! Thin transport gateway. Market interpretation, indexes, pagination and history live in core.
pub mod models;
mod remote;
use crate::client::CoreClient;
use crate::error::{ApiError, ErrorCode, Result};
use hub_protocol::wire as w;
pub use models::*;
pub(crate) use remote::{book_result as book_response, sync_response};
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonic::{
    Request, Status,
    metadata::{Ascii, MetadataValue},
};
#[derive(Clone)]
pub struct ApiState {
    pub node: CoreClient,
    pub(crate) streams: Arc<Semaphore>,
    queries: Arc<Semaphore>,
}
impl ApiState {
    pub fn new(node: CoreClient, max_queries: usize, max_streams: usize) -> Self {
        Self {
            node,
            queries: Arc::new(Semaphore::new(max_queries)),
            streams: Arc::new(Semaphore::new(max_streams)),
        }
    }
    pub fn stream_slot(&self) -> Result<OwnedSemaphorePermit> {
        self.streams.clone().try_acquire_owned().map_err(|_| {
            ApiError::new(
                ErrorCode::CapacityExceeded,
                "gateway stream capacity exhausted",
            )
        })
    }
    pub async fn call<T: Send + 'static, F, Fut>(&self, f: F) -> std::result::Result<T, Status>
    where
        F: FnOnce(
            w::market_read_client::MarketReadClient<tonic::transport::Channel>,
            MetadataValue<Ascii>,
        ) -> Fut,
        Fut: std::future::Future<Output = std::result::Result<T, Status>>,
    {
        let _permit = self
            .queries
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("gateway query capacity exhausted"))?;
        self.node
            .authenticated(|channel, auth| {
                f(
                    w::market_read_client::MarketReadClient::new(channel)
                        .max_decoding_message_size(4 * 1024 * 1024)
                        .max_encoding_message_size(64 * 1024),
                    auth,
                )
            })
            .await
    }
    pub async fn sync(
        &self,
        query: w::SyncRequest,
    ) -> std::result::Result<tonic::Streaming<w::SyncFrame>, Status> {
        self.replication(query, false).await
    }
    pub async fn replay(
        &self,
        query: w::SyncRequest,
    ) -> std::result::Result<tonic::Streaming<w::SyncFrame>, Status> {
        self.replication(query, true).await
    }
    async fn replication(
        &self,
        query: w::SyncRequest,
        replay: bool,
    ) -> std::result::Result<tonic::Streaming<w::SyncFrame>, Status> {
        self.node
            .authenticated(|channel, auth| {
                let mut rpc = w::market_read_client::MarketReadClient::new(channel)
                    .max_decoding_message_size(1024 * 1024);
                let mut request = Request::new(query.clone());
                request.metadata_mut().insert("authorization", auth);
                async move {
                    let response = tokio::time::timeout(Duration::from_secs(5), async {
                        if replay {
                            rpc.replay(request).await
                        } else {
                            rpc.sync(request).await
                        }
                    })
                    .await
                    .map_err(|_| Status::deadline_exceeded("core replication timed out"))??;
                    Ok(response.into_inner())
                }
            })
            .await
    }
}
pub fn request<T>(body: T, auth: MetadataValue<Ascii>) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert("authorization", auth);
    request.set_timeout(Duration::from_secs(5));
    request
}
