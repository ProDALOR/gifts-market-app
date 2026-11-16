use actix::Message;
use market_hub_protocol::wire as w;
use market_replica::ReadResult;
use tokio::sync::OwnedSemaphorePermit;
use tonic::Status;
#[derive(Message)]
#[rtype(result = "Result<ReadResult,Status>")]
pub(crate) struct ReadCatalog {
    pub request: w::ReadRequest,
    pub permit: OwnedSemaphorePermit,
}
#[derive(Message)]
#[rtype(result = "Result<Vec<crate::valuation::ResolvedPosition>,Status>")]
pub(crate) struct ResolveModels {
    pub positions: Vec<crate::valuation::ModelPosition>,
    pub permit: OwnedSemaphorePermit,
}
