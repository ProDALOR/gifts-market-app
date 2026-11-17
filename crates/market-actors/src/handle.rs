//! Stable transport-facing facade over responsibility-specific actor handles.
use crate::{
    catalog::handle::CatalogHandle,
    error,
    models::{
        BooksPage, BooksQuery, CatalogEntry, CatalogQuery, ExecutionsQuery, Order, OrdersQuery,
        Page, ReadConsistency,
    },
    orderbooks::handle::OrderbooksHandle,
    protocol,
    runtime::closed,
    selection,
    subscriptions::{DataSubscription, DataUpdate, UserSubscriptionsActor, messages::Subscribe},
};
use actix::Addr;
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
        request: crate::valuation::ValuationRequest,
    ) -> error::Result<crate::valuation::Valuation> {
        crate::valuation::validate(&request)?;
        let mut grouped = std::collections::BTreeMap::new();
        for position in request.positions {
            *grouped.entry(position.model).or_insert(0u32) += position.quantity;
        }
        let positions = grouped
            .into_iter()
            .map(|(model, quantity)| crate::valuation::ModelPosition { model, quantity })
            .collect();
        let resolved = self.catalog.resolve_models(positions).await?;
        Ok(self
            .orderbooks
            .value_models(resolved, request.currency_id)
            .await?)
    }
    pub(crate) async fn orders_raw(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        self.orderbooks.read(request).await
    }
    pub(crate) async fn catalog_raw(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        self.catalog.read(request).await
    }
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
    pub async fn orders(&self, query: OrdersQuery) -> error::Result<Page<Order>> {
        let mode = match query.consistency {
            ReadConsistency::Snapshot => w::ReadMode::Snapshot,
            ReadConsistency::Live => w::ReadMode::Live,
        };
        protocol::orders(self.orders_raw(query.into_read(mode)?).await?)
    }
    pub async fn executions(&self, query: ExecutionsQuery) -> error::Result<Page<Order>> {
        protocol::orders(self.orders_raw(query.into_read()?).await?)
    }
    pub async fn catalog(&self, query: CatalogQuery) -> error::Result<Page<CatalogEntry>> {
        protocol::catalog(self.catalog_raw(query.into_read()?).await?)
    }
    pub async fn books(&self, query: BooksQuery) -> error::Result<BooksPage> {
        let page = self.books_raw(query.into_request()?).await?;
        Ok(BooksPage {
            data: page.books.into_iter().map(Into::into).collect(),
            next_after: page.next_after.map(Into::into),
        })
    }
    pub async fn status(&self) -> error::Result<crate::SyncStatus> {
        Ok(self.status_raw().await?.into())
    }
    pub async fn subscribe(
        &self,
        query: OrdersQuery,
    ) -> error::Result<(Arc<DataSubscription>, watch::Receiver<DataUpdate>)> {
        Ok(self
            .subscribe_raw(query.into_read(w::ReadMode::Live)?)
            .await?)
    }
}
