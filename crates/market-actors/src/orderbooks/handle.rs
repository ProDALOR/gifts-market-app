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
}
impl OrderbooksHandle {
    pub async fn read(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        selection::orders(&request)?;
        let permit = self.budget.try_query()?;
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(ReadOrders { request, permit })
            .await
            .map_err(closed)?
    }
    pub async fn read_wait(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        selection::orders(&request)?;
        let permit = self.budget.wait().await?;
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(ReadOrders { request, permit })
            .await
            .map_err(closed)?
    }
    #[cfg(test)]
    pub async fn books(&self, request: w::BooksRequest) -> Result<w::BooksPage, Status> {
        let permit = self.budget.try_query()?;
        let _cancel = permit.cancel_on_drop();
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
