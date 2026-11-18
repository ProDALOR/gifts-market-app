#[cfg(test)]
use super::messages::ListBooks;
use super::{
    OrderbooksActor,
    messages::{GetStatus, ReadOrders},
};
use crate::{
    runtime::{QueryBudget, closed},
    selection,
};
use actix::Addr;
use market_hub_protocol::wire as w;
use market_replica::{ReadResult, SyncStatus};
use tonic::Status;
#[derive(Clone)]
pub(crate) struct OrderbooksHandle {
    pub address: Addr<OrderbooksActor>,
    pub budget: QueryBudget,
    pub valuations: std::sync::Arc<tokio::sync::Semaphore>,
}
impl OrderbooksHandle {
    pub async fn value_models(
        &self,
        request: market_gifts::valuation::ValuationRequest,
    ) -> Result<market_gifts::valuation::Valuation, Status> {
        self.valuation(request, false).await
    }
    pub async fn valuation(
        &self,
        request: market_gifts::valuation::ValuationRequest,
        wait: bool,
    ) -> Result<market_gifts::valuation::Valuation, Status> {
        let heavy = if wait {
            tokio::select! {biased;
                _=self.budget.stop.cancelled()=>return Err(Status::unavailable("Market data service is stopping")),
                permit=self.valuations.clone().acquire_owned()=>permit.map_err(|_|Status::unavailable("Valuation budget closed"))?,
            }
        } else {
            self.valuations
                .clone()
                .try_acquire_owned()
                .map_err(|_| Status::resource_exhausted("Portfolio valuation capacity reached"))?
        };
        let permit = self.budget.try_query()?;
        self.address
            .send(super::messages::ValueModels {
                request,
                permit,
                heavy,
            })
            .await
            .map_err(closed)?
    }
    pub async fn read(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        selection::orders(&request)?;
        let permit = self.budget.try_query()?;
        self.address
            .send(ReadOrders { request, permit })
            .await
            .map_err(closed)?
    }
    pub async fn read_wait(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        selection::orders(&request)?;
        let permit = self.budget.wait().await?;
        self.address
            .send(ReadOrders { request, permit })
            .await
            .map_err(closed)?
    }
    #[cfg(test)]
    pub async fn books(&self, request: w::BooksRequest) -> Result<w::BooksPage, Status> {
        let permit = self.budget.try_query()?;
        self.address
            .send(ListBooks { request, permit })
            .await
            .map_err(closed)?
    }
    pub async fn status(&self) -> Result<SyncStatus, Status> {
        self.budget.check()?;
        self.address.send(GetStatus).await.map_err(closed)?
    }
}
