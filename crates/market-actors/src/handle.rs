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
use market_replica::{ReadResult, SyncStatus as ReplicaStatus};
use std::sync::Arc;
use tokio::sync::watch;
use tonic::Status;
#[derive(Clone)]
pub struct MarketDataHandle {
    pub(crate) orderbooks: OrderbooksHandle,
    pub(crate) catalog: CatalogHandle,
    pub(crate) subscriptions: Addr<MarketSubscriptionsActor>,
}
impl MarketDataHandle {
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
}

impl MarketDataHandle {
    pub async fn subscribe_window(
        &self,
        query: market_gifts::GiftOrderWindowQuery,
    ) -> error::Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>)> {
        self.orderbooks.budget.check()?;
        query
            .validate()
            .map_err(|e| error::DataError::invalid(e.to_string()))?;
        let window = market_replica::WindowRequest {
            anchor: query.anchor,
            before: query.before,
            after: query.after,
            snapshot: None,
        };
        let mut orders = market_gifts::GiftOrdersQuery::new(query.filters);
        orders.side = query.side;
        orders.sort = query.sort;
        orders.limit = 200;
        let request = crate::query::orders(orders, w::ReadMode::Live)?;
        self.subscriptions
            .send(crate::subscriptions::messages::SubscribeWindow { request, window })
            .await
            .map_err(closed)?
            .map_err(Into::into)
    }

    pub async fn snapshot_window(
        &self,
        query: SnapshotWindowQuery,
    ) -> error::Result<SnapshotWindow> {
        query
            .query
            .validate()
            .map_err(|e| error::DataError::invalid(e.to_string()))?;
        let window = market_replica::WindowRequest {
            anchor: query.query.anchor,
            before: query.query.before,
            after: query.query.after,
            snapshot: query.snapshot,
        };
        let mut orders = GiftOrdersQuery::new(query.query.filters);
        orders.side = query.query.side;
        orders.sort = query.query.sort;
        orders.limit = 200;
        let result = self
            .orderbooks
            .window_wait(crate::query::orders(orders, w::ReadMode::Snapshot)?, window)
            .await?;
        let snapshot = result
            .snapshot
            .clone()
            .ok_or_else(|| error::DataError::unavailable("Missing snapshot lease"))?;
        let revision = result.revision.token();
        let (window, sync) = protocol::window(result)?;
        Ok(SnapshotWindow {
            window,
            snapshot,
            sync,
            revision,
        })
    }
    pub async fn release_window_snapshot(&self, snapshot: String) -> error::Result<()> {
        Ok(self.orderbooks.release_snapshot(snapshot).await?)
    }
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
}
