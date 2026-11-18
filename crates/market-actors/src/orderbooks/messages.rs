use actix::Message;
use market_hub_protocol::wire as w;
use market_replica::{ReadResult, SyncStatus};
use tonic::Status;
#[derive(Message)]
#[rtype(result = "Result<ReadResult,Status>")]
pub(crate) struct ReadOrders {
    pub request: w::ReadRequest,
    pub permit: market_replica::ReadPermit,
}
#[derive(Message)]
#[rtype(result = "Result<w::BooksPage,Status>")]
#[cfg(test)]
pub(crate) struct ListBooks {
    pub request: w::BooksRequest,
    pub permit: market_replica::ReadPermit,
}
#[derive(Message)]
#[rtype(result = "Result<SyncStatus,Status>")]
pub(crate) struct GetStatus;
