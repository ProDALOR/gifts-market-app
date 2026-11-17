use actix::Message;
use market_hub_protocol::wire as w;
use market_replica::{ReadResult, SyncStatus};
use tokio::sync::OwnedSemaphorePermit;
use tonic::Status;
#[derive(Message)]
#[rtype(result = "Result<ReadResult,Status>")]
pub(crate) struct ReadOrders {
    pub request: w::ReadRequest,
    pub permit: OwnedSemaphorePermit,
}
#[derive(Message)]
#[rtype(result = "Result<w::BooksPage,Status>")]
#[cfg(test)]
pub(crate) struct ListBooks {
    pub request: w::BooksRequest,
    pub permit: OwnedSemaphorePermit,
}
#[derive(Message)]
#[rtype(result = "Result<SyncStatus,Status>")]
pub(crate) struct GetStatus;
#[derive(Message)]
#[rtype(result = "Result<market_gifts::valuation::Valuation,Status>")]
pub(crate) struct ValueModels {
    pub heavy: OwnedSemaphorePermit,
    pub request: market_gifts::valuation::ValuationRequest,
    pub permit: OwnedSemaphorePermit,
}
