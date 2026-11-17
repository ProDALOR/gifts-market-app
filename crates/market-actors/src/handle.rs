//! Stable transport-facing facade over responsibility-specific actor handles.
use crate::{
    catalog::handle::CatalogHandle,
    error,
    orderbooks::handle::OrderbooksHandle,
    protocol,
    runtime::closed,
    selection,
    subscriptions::{DataSubscription, DataUpdate, UserSubscriptionsActor, messages::Subscribe},
};
use actix::Addr;
use market_gifts::*;
use market_hub_protocol::wire as w;
use market_replica::{ReadResult, SyncStatus as ReplicaStatus};
use std::sync::Arc;
use tokio::sync::watch;
use tonic::Status;
#[derive(Clone)]
pub struct MarketDataHandle {
    pub(crate) orderbooks: OrderbooksHandle,
    pub(crate) catalog: CatalogHandle,
    pub(crate) subscriptions: Addr<UserSubscriptionsActor>,
}
impl MarketDataHandle {
    pub async fn value_models(
        &self,
        request: market_gifts::valuation::ValuationRequest,
    ) -> error::Result<market_gifts::valuation::Valuation> {
        crate::query::valuation(&request)?;
        Ok(self.orderbooks.value_models(request).await?)
    }
    pub(crate) async fn orders_raw(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        self.orderbooks.read(request).await
    }
    #[cfg(test)]
    pub(crate) async fn catalog_raw(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        self.catalog.read(request).await
    }
    #[cfg(test)]
    pub(crate) async fn books_raw(&self, request: w::BooksRequest) -> Result<w::BooksPage, Status> {
        self.orderbooks.books(request).await
    }
    async fn status_raw(&self) -> Result<ReplicaStatus, Status> {
        self.orderbooks.status().await
    }
    pub(crate) async fn subscribe_raw(
        &self,
        mut request: w::ReadRequest,
    ) -> Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>), Status> {
        self.orderbooks.budget.check()?;
        selection::orders(&request)?;
        request.mode = w::ReadMode::Live as i32;
        self.subscriptions
            .send(Subscribe(request))
            .await
            .map_err(closed)?
    }
}

impl MarketDataHandle {
    pub async fn orders(&self, query: GiftOrdersQuery) -> error::Result<Page<GiftOrder>> {
        let mode = match query.consistency {
            ReadConsistency::Snapshot => w::ReadMode::Snapshot,
            ReadConsistency::Live => w::ReadMode::Live,
        };
        protocol::orders(self.orders_raw(crate::query::orders(query, mode)?).await?)
    }
    pub async fn executions(&self, query: GiftHistoryQuery) -> error::Result<Page<GiftOrder>> {
        protocol::orders(self.orders_raw(crate::query::history(query)?).await?)
    }
    pub async fn catalog(&self) -> error::Result<GiftCatalog> {
        Ok(self.catalog_snapshot().await?.data)
    }
    pub async fn collection(&self, id: CollectionId) -> error::Result<GiftCollectionCatalog> {
        Ok(self.collection_snapshot(id).await?.data)
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
        protocol::sync(self.status_raw().await?)
    }
    pub async fn subscribe(
        &self,
        query: GiftOrdersQuery,
    ) -> error::Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>)> {
        Ok(self
            .subscribe_raw(crate::query::orders(query, w::ReadMode::Live)?)
            .await?)
    }
}
