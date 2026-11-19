use actix::Message;
use market_hub_protocol::wire as w;
use market_replica::SyncStatus;
use tonic::Status;
#[derive(Message)]
#[rtype(result = "Result<SyncStatus,Status>")]
pub(crate) struct GetStatus;

#[derive(Message)]
#[rtype(result = "Result<(crate::OrderRead,market_replica::ReadRevision),Status>")]
pub(crate) struct ReadSlice {
    pub request: w::ReadRequest,
    pub slice: market_replica::SliceRequest,
    pub permit: market_replica::ReadPermit,
}

#[derive(Message)]
#[rtype(result = "Result<(),Status>")]
pub(crate) struct ReleaseSnapshot {
    pub snapshot: market_replica::SnapshotId,
}

#[derive(Message)]
#[rtype(result = "Result<(crate::OrderRead,market_replica::ReadRevision),Status>")]
pub(crate) struct ReadRange {
    pub request: w::ReadRequest,
    pub range: market_replica::KeyRange,
    pub permit: market_replica::ReadPermit,
}
