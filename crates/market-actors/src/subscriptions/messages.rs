use super::DataWatch;
use actix::Message;
use market_hub_protocol::wire as w;
use tonic::Status;
#[derive(Message)]
#[rtype(result = "Result<DataWatch,Status>")]
pub(crate) struct SubscribeCatalog(pub Vec<market_gifts::CollectionId>);

#[derive(Message)]
#[rtype(result = "Result<DataWatch,Status>")]
pub(crate) struct WatchOrders {
    pub request: w::ReadRequest,
    pub read: market_replica::RangeRequest,
    pub cancel: tokio_util::sync::CancellationToken,
}

#[derive(Message)]
#[rtype(result = "Result<DataWatch,Status>")]
pub(super) struct RegisterOrders {
    pub feed: super::types::OrdersFeed,
    pub cancel: tokio_util::sync::CancellationToken,
}
