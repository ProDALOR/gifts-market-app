//! Shared HTTP/WebSocket adapter over gift-domain actor capabilities.
use super::{error::Result, models::*};
use market_actors::{DataWatch, MarketDataHandle};
use market_gifts::{MediaTarget, SyncStatus, TelegramDocumentId};
use std::sync::Arc;
use tokio::sync::watch;
#[derive(Clone)]
pub struct MarketApi {
    pub(crate) portfolio: Option<market_portfolio::PortfolioHandle>,
    data: MarketDataHandle,
    valuation: market_valuation::ValuationHandle,
    encoder: Arc<crate::encoding::ResponseEncoder>,
}
impl MarketApi {
    pub(crate) async fn watch_valuation(
        &self,
        query: market_valuation::ValuationQuery,
    ) -> Result<market_valuation::ValuationWatch> {
        Ok(self.valuation.watch(query).await?)
    }
    pub async fn valuation(
        &self,
        query: market_valuation::ValuationQuery,
    ) -> Result<Arc<market_valuation::ValuationReport>> {
        Ok(self.valuation.query(query).await?)
    }
    pub(crate) fn status_updates(&self) -> watch::Receiver<SyncStatus> {
        self.data.status_updates()
    }
    pub fn new(data: MarketDataHandle, valuation: market_valuation::ValuationHandle) -> Self {
        Self::with_response_limit(
            data,
            valuation,
            crate::config::ApiLimits::default().response_bytes,
        )
    }
    pub(crate) fn with_response_limit(
        data: MarketDataHandle,
        valuation: market_valuation::ValuationHandle,
        limit: usize,
    ) -> Self {
        Self {
            data,
            valuation,
            portfolio: None,
            encoder: Arc::new(crate::encoding::ResponseEncoder::new(limit)),
        }
    }
    pub fn with_portfolio(mut self, portfolio: market_portfolio::PortfolioHandle) -> Self {
        self.portfolio = Some(portfolio);
        self
    }
    pub(crate) async fn encode_response<F>(&self, serialize: F) -> Result<actix_web::web::Bytes>
    where
        F: FnOnce(&mut crate::encoding::ResponseWriter) -> serde_json::Result<()> + Send + 'static,
    {
        self.encoder.encode(serialize).await
    }
    pub(crate) async fn watch_orders(
        &self,
        params: market_gifts::GiftOrderQuery,
    ) -> Result<DataWatch> {
        Ok(self.data.watch_orders(params).await?)
    }
    pub(crate) async fn shutdown(&self) {
        self.encoder.shutdown().await;
    }
    pub async fn ready(&self) -> Result<()> {
        if self.data.status().await?.ready {
            Ok(())
        } else {
            Err(market_actors::error::DataError::unavailable("Market data is loading").into())
        }
    }
    pub(crate) async fn catalog_snapshot(
        &self,
    ) -> Result<market_actors::CatalogSnapshot<market_gifts::GiftCatalog>> {
        Ok(self.data.catalog_snapshot().await?)
    }
    pub(crate) async fn collection_snapshot(
        &self,
        id: CollectionId,
    ) -> Result<market_actors::CatalogSnapshot<market_gifts::GiftCollectionCatalog>> {
        Ok(self.data.collection_snapshot(id).await?)
    }
    pub(crate) async fn subscribe_catalog(&self, ids: Vec<CollectionId>) -> Result<DataWatch> {
        Ok(self.data.subscribe_catalog(ids).await?)
    }
    pub(crate) async fn media_document(&self, target: MediaTarget) -> Result<TelegramDocumentId> {
        Ok(self.data.media_document(target).await?)
    }
    pub async fn query_orders(&self, query: GiftOrderQuery) -> Result<market_actors::OrderRead> {
        Ok(self.data.query_orders(query).await?)
    }
    pub(crate) async fn release_read_snapshot(
        &self,
        snapshot: market_gifts::ReadSnapshotId,
    ) -> Result<()> {
        Ok(self.data.release_read_snapshot(snapshot).await?)
    }
}
