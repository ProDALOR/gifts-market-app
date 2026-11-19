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
    /// Foreground reads reject overload; persistent producers have bounded waiting admission.
    pub async fn slice(
        &self,
        request: w::ReadRequest,
        slice: market_replica::SliceRequest,
    ) -> Result<(crate::OrderRead, market_replica::ReadRevision), Status> {
        self.read_slice(request, slice, self.budget.try_query()?)
            .await
    }
    pub async fn slice_wait(
        &self,
        request: w::ReadRequest,
        slice: market_replica::SliceRequest,
    ) -> Result<(crate::OrderRead, market_replica::ReadRevision), Status> {
        self.read_slice(request, slice, self.budget.wait().await?)
            .await
    }
    async fn read_slice(
        &self,
        request: w::ReadRequest,
        slice: market_replica::SliceRequest,
        permit: market_replica::ReadPermit,
    ) -> Result<(crate::OrderRead, market_replica::ReadRevision), Status> {
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(super::messages::ReadSlice {
                request,
                slice,
                permit,
            })
            .await
            .map_err(closed)?
    }

    pub async fn range_wait(
        &self,
        request: w::ReadRequest,
        range: market_replica::KeyRange,
    ) -> Result<(crate::OrderRead, market_replica::ReadRevision), Status> {
        let permit = self.budget.wait().await?;
        let _cancel = permit.cancel_on_drop();
        self.address
            .send(super::messages::ReadRange {
                request,
                range,
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
