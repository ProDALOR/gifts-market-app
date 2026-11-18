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
    pub(crate) catalog: CatalogHandle,
    pub(crate) subscriptions: Addr<MarketSubscriptionsActor>,
}
impl MarketDataHandle {
    pub async fn subscribe_window(
        &self,
        query: market_gifts::GiftOrderWindowQuery,
    ) -> error::Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>)> {
        self.orderbooks.budget.check()?;
        let (request, window) = crate::query::window(query, w::ReadMode::Live, None)?;
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
        let (request, window) =
            crate::query::window(query.query, w::ReadMode::Snapshot, query.snapshot)?;
        let result = self.orderbooks.window(request, window).await?;
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
        protocol::orders(
            self.orderbooks
                .read(crate::query::orders(query, mode)?)
                .await?,
        )
    }
    pub async fn executions(&self, query: GiftHistoryQuery) -> error::Result<Page<GiftOrder>> {
        protocol::orders(self.orderbooks.read(crate::query::history(query)?).await?)
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
