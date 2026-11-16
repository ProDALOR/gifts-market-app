//! Public API queries and responses mapped directly to the shared actor interface.
use super::{conversion, error::Result, models::*};
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
    pub(crate) async fn encode_page(
        &self,
        page: Arc<market_actors::models::Page<market_actors::models::Order>>,
    ) -> Result<Arc<str>> {
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
    pub async fn sync(&self) -> Result<SyncInfo> {
        Ok(self.data.status().await?.into())
    }
    pub async fn books(&self, query: BooksQuery) -> Result<BooksPage> {
        let page = self.data.books(query.into()).await?;
        Ok(BooksPage {
            data: page.data.into_iter().map(Into::into).collect(),
            next_after: page.next_after.map(Into::into),
        })
    }
    pub async fn catalog(&self, query: CatalogQuery) -> Result<Page<CatalogEntry>> {
        conversion::catalog(self.data.catalog(query.into()).await?)
    }
    pub async fn orders(&self, query: OrdersQuery) -> Result<Page<Order>> {
        Ok(conversion::orders(self.data.orders(query.into()).await?))
    }
    pub async fn executions(&self, query: ExecutionsQuery) -> Result<Page<Order>> {
        Ok(conversion::orders(
            self.data.executions(query.into()).await?,
        ))
    }
    pub async fn subscribe(
        &self,
        query: OrdersQuery,
    ) -> Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>)> {
        Ok(self.data.subscribe(query.into()).await?)
    }
}
