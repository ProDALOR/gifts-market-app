use super::{OrderbooksActor, messages::GetStatus};
use crate::runtime::{QueryBudget, closed};
use actix::Addr;
use market_hub_protocol::wire as w;
use market_replica::SyncStatus;
use tonic::Status;
#[derive(Clone)]
pub(crate) struct OrderbooksHandle {
    pub address: Addr<OrderbooksActor>,
    pub budget: QueryBudget,
}
impl OrderbooksHandle {
    /// Foreground queries reject overload; persistent watches wait within shared admission.
    pub async fn query(
        &self,
        request: w::ReadRequest,
        read: market_replica::RangeRequest,
    ) -> Result<crate::OrderSnapshot, Status> {
        self.read_admitted(request, read, false, self.budget.try_query()?)
            .await
    }
    pub async fn range_wait(
        &self,
        request: w::ReadRequest,
        read: market_replica::RangeRequest,
        complete: bool,
    ) -> Result<crate::OrderSnapshot, Status> {
        self.read_admitted(request, read, complete, self.budget.wait().await?)
            .await
    }
    pub(crate) async fn read_admitted(
        &self,
        request: w::ReadRequest,
        read: market_replica::RangeRequest,
        complete: bool,
        permit: market_replica::ReadPermit,
    ) -> Result<crate::OrderSnapshot, Status> {
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(super::messages::ReadRange {
                request,
                read,
                complete,
                permit,
            })
            .await
            .map_err(closed)?
    }
    pub async fn release_snapshot(
        &self,
        snapshot: market_replica::SnapshotId,
    ) -> Result<(), Status> {
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
