use super::{
    OrderbooksActor,
    messages::{GetStatus, ReadOrders},
};
use crate::runtime::{QueryBudget, closed};
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
    /// Foreground reads reject overload; persistent producers have bounded waiting admission.
    pub async fn window(
        &self,
        request: w::ReadRequest,
        window: market_replica::WindowRequest,
    ) -> Result<market_replica::WindowReadResult, Status> {
        self.read_window(request, window, self.budget.try_query()?)
            .await
    }
    pub async fn window_wait(
        &self,
        request: w::ReadRequest,
        window: market_replica::WindowRequest,
    ) -> Result<market_replica::WindowReadResult, Status> {
        self.read_window(request, window, self.budget.wait().await?)
            .await
    }
    async fn read_window(
        &self,
        request: w::ReadRequest,
        window: market_replica::WindowRequest,
        permit: market_replica::ReadPermit,
    ) -> Result<market_replica::WindowReadResult, Status> {
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(super::messages::ReadWindow {
                request,
                window,
                permit,
            })
            .await
            .map_err(closed)?
    }

    pub async fn read(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        let permit = self.budget.try_query()?;
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(ReadOrders { request, permit })
            .await
            .map_err(closed)?
    }
    pub async fn release_snapshot(&self, snapshot: String) -> Result<(), Status> {
        self.budget.check()?;
        self.address
            .send(super::messages::ReleaseSnapshot { snapshot })
            .await
            .map_err(closed)?
    }
    pub async fn status(&self) -> Result<SyncStatus, Status> {
        self.budget.check()?;
        self.address.send(GetStatus).await.map_err(closed)?
    }
}
