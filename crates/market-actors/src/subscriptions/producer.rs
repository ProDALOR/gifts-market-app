//! Shared order-page computation; subscriber registration belongs to the actor.
use super::{DataUpdate, QueryFeed};
use crate::{
    error::DataError,
    models::{Order, Page},
};
use crate::{orderbooks::handle::OrderbooksHandle, runtime::ActorEnvironment};
use market_hub_protocol::wire as w;
use std::{
    sync::{Arc, Weak, atomic::Ordering},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, watch};
use tonic::Status;
struct Completion {
    feed: Weak<QueryFeed>,
    sender: watch::Sender<DataUpdate>,
}
impl Drop for Completion {
    fn drop(&mut self) {
        if let Some(feed) = self.feed.upgrade() {
            feed.done.store(true, Ordering::Release);
        }
        if !matches!(*self.sender.borrow(), DataUpdate::Error(_)) {
            self.sender.send_replace(DataUpdate::Error(
                Status::unavailable("Live subscription stopped").into(),
            ));
        }
    }
}
pub(super) fn start(
    environment: &ActorEnvironment,
    feed: &Arc<QueryFeed>,
    orders: OrderbooksHandle,
    request: w::ReadRequest,
    mut changed: watch::Receiver<u64>,
    permit: OwnedSemaphorePermit,
) {
    let sender = feed.sender.clone();
    let stop = feed.stop.clone();
    let completion = Completion {
        feed: Arc::downgrade(feed),
        sender: sender.clone(),
    };
    environment.jobs.spawn_on(async move {
        let _completion=completion;let _permit=permit;
        let work=async {
            let mut previous:Option<Arc<Page<Order>>>=None;
            loop {
                changed.borrow_and_update();
                let result=tokio::select! {biased;_=stop.cancelled()=>return Ok(()),_=sender.closed()=>return Ok(()),result=orders.read_wait(request.clone())=>result?};
                let page=Arc::new(crate::protocol::orders(result)?);
                if previous.as_ref().is_none_or(|old|old.data!=page.data || old.next_page_token!=page.next_page_token || old.sync!=page.sync) {
                    previous=Some(page.clone());sender.send_replace(DataUpdate::Page(page));
                }
                tokio::select! {
                    _=stop.cancelled()=>return Ok(()),
                    _=sender.closed()=>return Ok(()),
                    result=changed.changed()=>{result.map_err(|_|Status::unavailable("Replica updates stopped"))?;},
                    _=tokio::time::sleep(Duration::from_secs(10))=>previous=None,
                }
                tokio::select! {_=stop.cancelled()=>return Ok(()),_=sender.closed()=>return Ok::<(),DataError>(()),_=tokio::time::sleep(Duration::from_millis(100))=>{}}
            }
        };
        if let Err(error)=work.await {sender.send_replace(DataUpdate::Error(error));}
    },&environment.runtime);
}
