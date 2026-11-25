//! Stable transport-facing facade over responsibility-specific actor handles.
use crate::{
    catalog::handle::CatalogHandle,
    error,
    orderbooks::handle::OrderbooksHandle,
    protocol,
    runtime::closed,
    subscriptions::{DataWatch, MarketSubscriptionsActor},
};
use actix::Addr;
use market_gifts::*;
use market_gifts_hub::input::{CollectionRef, GiftFilters, GiftOrderQuery};
use tokio::sync::watch;
#[derive(Clone)]
pub struct MarketDataHandle {
    pub(crate) orderbooks: OrderbooksHandle,
    pub(crate) status_updates: watch::Receiver<SyncStatus>,
    pub(crate) catalog: CatalogHandle,
    pub(crate) subscriptions: Addr<MarketSubscriptionsActor>,
}
impl MarketDataHandle {
    pub async fn resolve_filters(
        &self,
        filters: GiftFilters,
    ) -> error::Result<ResolvedGiftFilters> {
        Ok(self.catalog.resolve_filters(filters).await?)
    }

    pub fn status_updates(&self) -> watch::Receiver<SyncStatus> {
        self.status_updates.clone()
    }
    pub async fn watch_orders(&self, query: GiftOrderQuery) -> error::Result<DataWatch> {
        self.orderbooks.budget.check()?;
        let (request, read) = crate::query::compile(query)?;
        if !request.is_live() {
            return Err(error::DataError::invalid("watches require latest reads"));
        }
        let cancel = tokio_util::sync::CancellationToken::new();
        let _cancel = cancel.clone().drop_guard();
        self.subscriptions
            .send(crate::subscriptions::messages::WatchOrders {
                request,
                read,
                cancel,
            })
            .await
            .map_err(closed)?
            .map_err(Into::into)
    }
    pub async fn query_orders(&self, query: GiftOrderQuery) -> error::Result<crate::OrderRead> {
        let (request, read) = crate::query::compile(query)?;
        Ok(self.orderbooks.query(request, read).await?.read)
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
        id: CollectionRef,
    ) -> error::Result<crate::CatalogSnapshot<GiftCollectionCatalog>> {
        Ok(self.catalog.collection(id).await?)
    }
    pub async fn subscribe_catalog(&self, ids: Vec<CollectionRef>) -> error::Result<DataWatch> {
        self.catalog.budget.check()?;
        Ok(self
            .subscriptions
            .send(crate::subscriptions::messages::SubscribeCatalog(ids))
            .await
            .map_err(closed)??)
    }
    pub async fn media_document(&self, target: MediaTarget) -> error::Result<TelegramDocumentId> {
        Ok(self.catalog.media_document(target).await?)
    }
    pub async fn status(&self) -> error::Result<SyncStatus> {
        protocol::sync(self.orderbooks.status().await?)
    }
}
