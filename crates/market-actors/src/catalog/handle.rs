use super::{CatalogActor, messages::ReadCatalog};
use crate::{
    runtime::{QueryBudget, closed},
    selection,
};
use actix::Addr;
use market_hub_protocol::wire as w;
use market_replica::ReadResult;
use tonic::Status;
#[derive(Clone)]
pub(crate) struct CatalogHandle {
    pub address: Addr<CatalogActor>,
    pub budget: QueryBudget,
}
impl CatalogHandle {
    pub async fn resolve_models(
        &self,
        positions: Vec<crate::valuation::ModelPosition>,
    ) -> Result<Vec<crate::valuation::ResolvedPosition>, Status> {
        let permit = self.budget.try_query()?;
        self.address
            .send(super::messages::ResolveModels { positions, permit })
            .await
            .map_err(closed)?
    }
    pub async fn read(&self, request: w::ReadRequest) -> Result<ReadResult, Status> {
        selection::catalog(&request)?;
        let permit = self.budget.try_query()?;
        self.address
            .send(ReadCatalog { request, permit })
            .await
            .map_err(closed)?
    }
}
