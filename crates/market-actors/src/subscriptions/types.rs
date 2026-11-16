use super::{UserSubscriptionsActor, messages::Release};
use crate::{
    error::DataError,
    models::{Order, Page},
};
use actix::Addr;
use std::sync::{Arc, atomic::AtomicBool};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub enum DataUpdate {
    Connecting,
    Page(Arc<Page<Order>>),
    Error(DataError),
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct SubscriberId(pub u64);
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct QueryKey(pub Vec<u8>);
/// One caller's lease. Dropping its final Arc releases only that caller.
pub struct DataSubscription {
    pub(super) id: SubscriberId,
    pub(super) owner: Addr<UserSubscriptionsActor>,
}
impl Drop for DataSubscription {
    fn drop(&mut self) {
        // Cleanup must be delivered even when the ordinary request mailbox is full.
        self.owner.do_send(Release(self.id));
    }
}
pub(super) struct QueryFeed {
    pub sender: watch::Sender<DataUpdate>,
    pub done: AtomicBool,
    pub stop: CancellationToken,
}
