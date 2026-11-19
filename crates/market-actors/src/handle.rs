//! Stable transport-facing facade over responsibility-specific actor handles.
use crate::{
    catalog::handle::CatalogHandle,
    error,
    orderbooks::handle::OrderbooksHandle,
    protocol,
    runtime::closed,
    subscriptions::{DataSubscription, DataUpdate, MarketSubscriptionsActor},
};
use actix::Addr;
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::sync::Arc;
use tokio::sync::watch;
#[derive(Clone)]
pub struct MarketDataHandle {
    pub(crate) orderbooks: OrderbooksHandle,
    pub(crate) status_updates: watch::Receiver<SyncStatus>,
    pub(crate) catalog: CatalogHandle,
    pub(crate) subscriptions: Addr<MarketSubscriptionsActor>,
}
impl MarketDataHandle {
    pub fn status_updates(&self) -> watch::Receiver<SyncStatus> {
        self.status_updates.clone()
    }
    pub async fn watch_orders(
        &self,
        query: OrderWatchQuery,
    ) -> error::Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>)> {
        self.orderbooks.budget.check()?;
        let (request, slice) = crate::query::selection(query.query)?;
        let range = crate::query::range(query.range.as_ref())?;
        if request.mode != w::ReadMode::Live as i32 {
            return Err(error::DataError::invalid("watches require latest reads"));
        }
        self.subscriptions
            .send(crate::subscriptions::messages::WatchOrders {
                request,
                slice,
                range,
            })
            .await
            .map_err(closed)?
            .map_err(Into::into)
    }
    pub async fn query_orders(&self, query: GiftOrderQuery) -> error::Result<crate::OrderRead> {
        let (request, slice) = crate::query::selection(query)?;
        Ok(self.orderbooks.slice(request, slice).await?.0)
    }
    pub async fn release_read_snapshot(&self, snapshot: ReadSnapshotId) -> error::Result<()> {
        Ok(self
            .orderbooks
            .release_snapshot(snapshot.as_str().parse()?)
            .await?)
    }
    pub async fn catalog_snapshot(&self) -> error::Result<crate::CatalogSnapshot<GiftCatalog>> {
        Ok(self.catalog.bootstrap().await?)
    }
    pub async fn collection_snapshot(
        &self,
        id: CollectionId,
    ) -> error::Result<crate::CatalogSnapshot<GiftCollectionCatalog>> {
        Ok(self.catalog.collection(id).await?)
    }
    pub async fn subscribe_catalog(
        &self,
        mut ids: Vec<CollectionId>,
    ) -> error::Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>)> {
        self.catalog.budget.check()?;
        if ids.len() > 8 {
            return Err(error::DataError::invalid("Watch at most eight collections"));
        }
        ids.sort();
        ids.dedup();
        Ok(self
            .subscriptions
            .send(crate::subscriptions::messages::SubscribeCatalog(ids))
            .await
            .map_err(closed)??)
    }
    pub async fn media_document(&self, target: MediaTarget) -> error::Result<TelegramDocumentId> {
        Ok(self.catalog.media_document(target).await?)
    }
    /// Return at most `limit` distinct model documents (1..=1000).
    pub async fn model_documents(&self, limit: usize) -> error::Result<Vec<TelegramDocumentId>> {
        Ok(self.catalog.model_documents(limit).await?)
    }
    pub async fn status(&self) -> error::Result<SyncStatus> {
        protocol::sync(self.orderbooks.status().await?)
    }
}
