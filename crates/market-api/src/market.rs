//! Shared HTTP/WebSocket adapter over gift-domain actor capabilities.
use super::{error::Result, models::*};
use market_actors::{DataWatch, MarketDataHandle};
use market_gifts::{MediaTarget, SyncStatus, TelegramDocumentId};
use std::sync::Arc;
use tokio::sync::watch;
#[derive(Clone)]
pub struct MarketApi {
    data: MarketDataHandle,
    valuation: market_valuation::ValuationHandle,
    encoder: Arc<crate::encoding::OrderEncoder>,
}
impl MarketApi {
    pub async fn market_values(
        &self,
        query: market_valuation::MarketValuesRequest,
    ) -> Result<Arc<market_valuation::MarketValues>> {
        Ok(self.valuation.values(query).await?)
    }
    pub(crate) async fn watch_market_values(
        &self,
        query: market_valuation::MarketValuesRequest,
    ) -> Result<market_valuation::MarketValuesWatch> {
        Ok(self.valuation.watch_values(query).await?)
    }
    pub async fn target_valuation(
        &self,
        query: market_valuation::TargetValuationRequest,
    ) -> Result<Arc<market_valuation::TargetValuationReport>> {
        Ok(self.valuation.query(query).await?)
    }
    pub(crate) fn status_updates(&self) -> watch::Receiver<SyncStatus> {
        self.data.status_updates()
    }
    pub fn new(data: MarketDataHandle, valuation: market_valuation::ValuationHandle) -> Self {
        Self::with_order_limit(
            data,
            valuation,
            crate::config::WebSocketLimits::default().order_bytes,
        )
    }
    pub(crate) fn with_order_limit(
        data: MarketDataHandle,
        valuation: market_valuation::ValuationHandle,
        limit: usize,
    ) -> Self {
        Self {
            data,
            valuation,
            encoder: Arc::new(crate::encoding::OrderEncoder::new(limit)),
        }
    }
    pub(crate) async fn encode_orders(
        &self,
        event: crate::encoding::OrderEvent,
    ) -> Result<Arc<str>> {
        self.encoder.encode(event).await
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
    pub async fn gift_valuation(
        &self,
        query: market_valuation::GiftValuationRequest,
    ) -> Result<Arc<market_valuation::GiftFloorMark>> {
        Ok(self.valuation.gift(query).await?)
    }
}
