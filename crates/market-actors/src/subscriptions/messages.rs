use super::{DataSubscription, DataUpdate};
use actix::Message;
use market_hub_protocol::wire as w;
use std::sync::Arc;
use tokio::sync::watch;
use tonic::Status;
#[derive(Message)]
#[rtype(result = "Result<(Arc<DataSubscription>,watch::Receiver<DataUpdate>),Status>")]
pub(crate) struct SubscribeCatalog(pub Vec<market_gifts::CollectionId>);

#[derive(Message)]
#[rtype(result = "Result<(Arc<DataSubscription>,watch::Receiver<DataUpdate>),Status>")]
pub(crate) struct WatchOrders {
    pub request: w::ReadRequest,
    pub read: market_replica::RangeRequest,
}
