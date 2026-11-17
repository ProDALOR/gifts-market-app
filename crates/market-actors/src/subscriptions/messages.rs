use super::{DataSubscription, DataUpdate, SubscriberId};
use actix::Message;
use market_hub_protocol::wire as w;
use std::sync::Arc;
use tokio::sync::watch;
use tonic::Status;
#[derive(Message)]
#[rtype(result = "Result<(Arc<DataSubscription>,watch::Receiver<DataUpdate>),Status>")]
pub(crate) struct Subscribe(pub w::ReadRequest);
#[derive(Message)]
#[rtype(result = "()")]
pub(super) struct Release(pub SubscriberId);

#[derive(Message)]
#[rtype(result = "Result<(Arc<DataSubscription>,watch::Receiver<DataUpdate>),Status>")]
pub(crate) struct SubscribeCatalog(pub Vec<market_gifts::CollectionId>);
