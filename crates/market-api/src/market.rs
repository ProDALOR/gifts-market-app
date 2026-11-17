//! Thin HTTP-facing adapter over gift-domain actor capabilities.
use super::{error::Result, models::*};
use market_actors::{DataSubscription, DataUpdate, MarketDataHandle};
use std::sync::Arc;
use tokio::sync::watch;
#[derive(Clone)]
pub struct MarketApi {
    data: MarketDataHandle,
    encoder: Arc<crate::encoding::PageEncoder>,
}
impl MarketApi {
    pub fn new(data: MarketDataHandle) -> Self {
        Self::with_page_limit(data, crate::config::WebSocketLimits::default().page_bytes)
    }
    pub(crate) fn with_page_limit(data: MarketDataHandle, limit: usize) -> Self {
        Self {
            data,
            encoder: Arc::new(crate::encoding::PageEncoder::new(limit)),
        }
    }
    pub(crate) async fn encode_page(&self, page: Arc<Page<GiftOrder>>) -> Result<Arc<str>> {
        self.encoder.encode(page).await
    }
    pub(crate) async fn shutdown(&self) {
        self.encoder.shutdown().await;
    }
    pub async fn ready(&self) -> Result<()> {
        if self.data.status().await?.ready {
            Ok(())
        } else {
            Err(market_actors::error::DataError::unavailable("Market replica is loading").into())
        }
    }
    pub async fn sync(&self) -> Result<SyncStatus> {
        Ok(self.data.status().await?)
    }
    pub async fn catalog(&self) -> Result<GiftCatalog> {
        Ok(self.data.catalog().await?)
    }
    pub async fn collection(&self, id: CollectionId) -> Result<GiftCollectionCatalog> {
        Ok(self.data.collection(id).await?)
    }
    pub(crate) async fn media_document(&self, target: MediaTarget) -> Result<TelegramDocumentId> {
        Ok(self.data.media_document(target).await?)
    }
    pub async fn orders(&self, query: GiftOrdersQuery) -> Result<Page<GiftOrder>> {
        Ok(self.data.orders(query).await?)
    }
    pub async fn executions(&self, query: GiftHistoryQuery) -> Result<Page<GiftOrder>> {
        Ok(self.data.executions(query).await?)
    }
    pub async fn value_models(
        &self,
        query: market_gifts::valuation::ValuationRequest,
    ) -> Result<market_gifts::valuation::Valuation> {
        Ok(self.data.value_models(query).await?)
    }
    pub async fn subscribe(
        &self,
        query: GiftOrdersQuery,
    ) -> Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>)> {
        Ok(self.data.subscribe(query).await?)
    }
}
