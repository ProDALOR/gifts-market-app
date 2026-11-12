//! Downstream read RPCs proxy to core with the gateway's own node credential.
use crate::query::{ApiState, request};
use futures_util::StreamExt;
use hub_protocol::wire as w;
use std::pin::Pin;
use tokio::net::TcpListener;
use tokio_stream::{Stream, wrappers::TcpListenerStream};
use tokio_util::sync::CancellationToken;
use tonic::{
    Request, Response, Status,
    transport::{Server, ServerTlsConfig},
};
type Output<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;
#[derive(Clone)]
struct Service {
    state: ApiState,
    stop: CancellationToken,
}
#[tonic::async_trait]
impl w::market_read_server::MarketRead for Service {
    async fn get_status(&self, r: Request<w::Empty>) -> Result<Response<w::ReadStatus>, Status> {
        let value = r.into_inner();
        let result = self
            .state
            .call(|mut rpc, auth| async move {
                Ok(rpc.get_status(request(value, auth)).await?.into_inner())
            })
            .await?;
        Ok(Response::new(result))
    }
    async fn list_catalog(
        &self,
        r: Request<w::CatalogQuery>,
    ) -> Result<Response<w::CatalogResult>, Status> {
        let value = r.into_inner();
        let result = self
            .state
            .call(|mut rpc, auth| {
                let value = value.clone();
                async move { Ok(rpc.list_catalog(request(value, auth)).await?.into_inner()) }
            })
            .await?;
        Ok(Response::new(result))
    }
    async fn list_feeds(
        &self,
        r: Request<w::FeedsQuery>,
    ) -> Result<Response<w::FeedsResult>, Status> {
        let value = r.into_inner();
        let result = self
            .state
            .call(|mut rpc, auth| {
                let value = value.clone();
                async move { Ok(rpc.list_feeds(request(value, auth)).await?.into_inner()) }
            })
            .await?;
        Ok(Response::new(result))
    }
    async fn list_items(
        &self,
        r: Request<w::ItemsQuery>,
    ) -> Result<Response<w::ItemsResult>, Status> {
        let value = r.into_inner();
        let result = self
            .state
            .call(|mut rpc, auth| {
                let value = value.clone();
                async move { Ok(rpc.list_items(request(value, auth)).await?.into_inner()) }
            })
            .await?;
        Ok(Response::new(result))
    }
    async fn list_orders(
        &self,
        r: Request<w::OrdersQuery>,
    ) -> Result<Response<w::OrdersResult>, Status> {
        let value = r.into_inner();
        let result = self
            .state
            .call(|mut rpc, auth| {
                let value = value.clone();
                async move { Ok(rpc.list_orders(request(value, auth)).await?.into_inner()) }
            })
            .await?;
        Ok(Response::new(result))
    }
    async fn get_order_book(
        &self,
        r: Request<w::BookQuery>,
    ) -> Result<Response<w::BookResult>, Status> {
        let value = r.into_inner();
        let result = self
            .state
            .call(|mut rpc, auth| {
                let value = value.clone();
                async move { Ok(rpc.get_order_book(request(value, auth)).await?.into_inner()) }
            })
            .await?;
        Ok(Response::new(result))
    }
    async fn list_stored_orders(
        &self,
        r: Request<w::OrdersQuery>,
    ) -> Result<Response<w::OrdersResult>, Status> {
        let value = r.into_inner();
        let result = self
            .state
            .call(|mut rpc, auth| {
                let value = value.clone();
                async move {
                    Ok(rpc
                        .list_stored_orders(request(value, auth))
                        .await?
                        .into_inner())
                }
            })
            .await?;
        Ok(Response::new(result))
    }
    async fn list_trades(
        &self,
        r: Request<w::TradesQuery>,
    ) -> Result<Response<w::TradesResult>, Status> {
        let value = r.into_inner();
        let result = self
            .state
            .call(|mut rpc, auth| {
                let value = value.clone();
                async move { Ok(rpc.list_trades(request(value, auth)).await?.into_inner()) }
            })
            .await?;
        Ok(Response::new(result))
    }
    type WatchOrderBookStream = Output<w::BookResult>;
    async fn watch_order_book(
        &self,
        r: Request<w::BookQuery>,
    ) -> Result<Response<Self::WatchOrderBookStream>, Status> {
        let permit = self.state.stream_slot().map_err(Status::from)?;
        let value = r.into_inner();
        let stream = self
            .state
            .call(|mut rpc, auth| {
                let value = value.clone();
                async move {
                    let mut r = Request::new(value);
                    r.metadata_mut().insert("authorization", auth);
                    Ok(tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        rpc.watch_order_book(r),
                    )
                    .await
                    .map_err(|_| Status::deadline_exceeded("core subscription timed out"))??
                    .into_inner())
                }
            })
            .await?;
        let stop = self.stop.clone();
        Ok(Response::new(Box::pin(async_stream::try_stream! {
            let _permit=permit;let mut stream=Box::pin(stream.take_until(stop.cancelled_owned()));
            while let Some(frame)=stream.next().await {yield frame?;}
        })))
    }
    type SyncStream = Output<w::SyncFrame>;
    async fn sync(&self, r: Request<w::SyncRequest>) -> Result<Response<Self::SyncStream>, Status> {
        let permit = self.state.stream_slot().map_err(Status::from)?;
        let stream = self.state.sync(r.into_inner()).await?;
        let stop = self.stop.clone();
        Ok(Response::new(Box::pin(async_stream::try_stream! {
            let _permit=permit;let mut stream=Box::pin(stream.take_until(stop.cancelled_owned()));
            while let Some(frame)=stream.next().await {yield frame?;}
        })))
    }
    type ReplayStream = Output<w::SyncFrame>;
    async fn replay(
        &self,
        r: Request<w::SyncRequest>,
    ) -> Result<Response<Self::ReplayStream>, Status> {
        let permit = self.state.stream_slot().map_err(Status::from)?;
        let stream = self.state.replay(r.into_inner()).await?;
        let stop = self.stop.clone();
        Ok(Response::new(Box::pin(async_stream::try_stream! {
            let _permit=permit;let mut stream=Box::pin(stream.take_until(stop.cancelled_owned()));
            while let Some(frame)=stream.next().await {yield frame?;}
        })))
    }
}
pub async fn serve(
    state: ApiState,
    listener: TcpListener,
    tls: Option<ServerTlsConfig>,
    token: Option<String>,
    stop: CancellationToken,
) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    use subtle::ConstantTimeEq;
    let expected = token.map(|token| Sha256::digest(token.as_bytes()));
    let auth = move |r: Request<()>| -> Result<Request<()>, Status> {
        if let Some(expected) = expected {
            let supplied = r
                .metadata()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .unwrap_or_default();
            if !bool::from(expected.ct_eq(&Sha256::digest(supplied.as_bytes()))) {
                return Err(Status::unauthenticated("invalid API credential"));
            }
        }
        Ok(r)
    };
    let service = Service {
        state,
        stop: stop.clone(),
    };
    let reads = w::market_read_server::MarketReadServer::new(service.clone())
        .max_decoding_message_size(64 * 1024)
        .max_encoding_message_size(4 * 1024 * 1024);
    let mut server = Server::builder()
        .concurrency_limit_per_connection(16)
        .max_concurrent_streams(256)
        .load_shed(true);
    if let Some(tls) = tls {
        server = server.tls_config(tls)?;
    }
    server
        .add_service(tonic::service::interceptor::InterceptedService::new(
            reads, auth,
        ))
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), stop.cancelled_owned())
        .await?;
    Ok(())
}
