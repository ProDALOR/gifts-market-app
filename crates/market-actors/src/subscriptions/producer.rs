//! Live window and catalog computation; subscriber registration belongs to the actor.
use super::{DataUpdate, QueryFeed};
use crate::error::DataError;
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
/// Catalog notifications use the replica's metadata clock, never order-price or freshness ticks.
pub(super) fn start_catalog(
    environment: &ActorEnvironment,
    feed: &Arc<QueryFeed>,
    catalog: crate::catalog::handle::CatalogHandle,
    ids: Vec<market_gifts::CollectionId>,
    reader: market_replica::ReplicaReader,
    permit: OwnedSemaphorePermit,
) {
    let sender = feed.sender.clone();
    let stop = feed.stop.clone();
    let completion = Completion {
        feed: Arc::downgrade(feed),
        sender: sender.clone(),
    };
    let work = async move {
        let _completion = completion;
        let _permit = permit;
        let mut changed = reader.changes();
        let work = async {
            let mut previous = None;
            let mut revision = None;
            loop {
                changed.borrow_and_update();
                let current = (
                    reader.metadata_revision(&[
                        w::EntityKind::ItemType,
                        w::EntityKind::AttributeOption,
                        w::EntityKind::Venue,
                        w::EntityKind::Currency,
                    ])?,
                    reader.directory_revision()?,
                );
                if revision != Some(current) {
                    let versions = tokio::select! { biased;
                        _ = stop.cancelled() => return Ok::<(), DataError>(()),
                        _ = sender.closed() => return Ok(()),
                        result = catalog.versions(ids.clone()) => result?,
                    };
                    if previous.as_ref() != Some(&versions) {
                        previous = Some(versions.clone());
                        sender.send_replace(DataUpdate::Catalog(versions));
                    }
                    revision = Some(current);
                }
                tokio::select! {
                    _ = stop.cancelled() => return Ok(()),
                    _ = sender.closed() => return Ok(()),
                    result = changed.changed() => result.map_err(|_| Status::unavailable("Replica updates stopped"))?,
                }
                tokio::select! {
                    _ = stop.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                }
            }
        };
        if let Err(error) = work.await {
            sender.send_replace(DataUpdate::Error(error));
        }
    };
    environment.jobs.spawn_on(work, &environment.runtime);
}

pub(super) fn start_orders(
    environment: &ActorEnvironment,
    feed: &Arc<QueryFeed>,
    orders: OrderbooksHandle,
    selection: super::types::OrdersFeed,
    reader: market_replica::ReplicaReader,
    permit: OwnedSemaphorePermit,
) {
    let super::types::OrdersFeed {
        request,
        window,
        range: mut coverage,
    } = selection;
    let sender = feed.sender.clone();
    let stop = feed.stop.clone();
    let completion = Completion {
        feed: Arc::downgrade(feed),
        sender: sender.clone(),
    };
    let work = async move {
        let _completion = completion;
        let _permit = permit;
        let mut changed = reader.changes();
        let work = async {
            let mut previous: Option<Arc<market_gifts::OrderResult>> = None;
            let mut revision = None;
            let mut freshness = None;
            loop {
                changed.borrow_and_update();
                let current = reader.read_revision(&request)?;
                if revision != Some(current) {
                    let read = async {
                        if let Some(range) = &coverage {
                            let range = market_replica::WindowRange {
                                start: range.start.as_ref().map(|b| b.position.clone()),
                                end: range.end.as_ref().map(|b| b.position.clone()),
                            };
                            orders.range_wait(request.clone(), range).await
                        } else {
                            orders.window_wait(request.clone(), window.clone()).await
                        }
                    };
                    let result = tokio::select! {biased;_=stop.cancelled()=>return Ok(()),_=sender.closed()=>return Ok(()),result=read=>result?};
                    let current = result.revision;
                    let (next, sync) = crate::protocol::order_result(result, coverage.clone())?;
                    coverage = Some(next.range.clone());
                    if previous.as_ref().is_none_or(|old| **old != next) {
                        previous = Some(Arc::new(next));
                    }
                    revision = Some(current);
                    freshness = Some(sync.clone());
                    sender.send_replace(DataUpdate::Orders {
                        epoch: current.dataset_token(),
                        result: previous.as_ref().expect("window loaded").clone(),
                        sync,
                        revision: current.token(),
                    });
                } else {
                    let sync = crate::protocol::sync(reader.status())?;
                    if freshness.as_ref() != Some(&sync) {
                        freshness = Some(sync.clone());
                        sender.send_replace(DataUpdate::Orders {
                            epoch: revision.expect("revision loaded").dataset_token(),
                            result: previous.as_ref().expect("window loaded").clone(),
                            sync,
                            revision: revision.expect("revision loaded").token(),
                        });
                    }
                }
                tokio::select! {_=stop.cancelled()=>return Ok(()),_=sender.closed()=>return Ok(()),result=changed.changed()=>result.map_err(|_|Status::unavailable("Replica updates stopped"))?}
                tokio::select! {_=stop.cancelled()=>return Ok(()),_=tokio::time::sleep(Duration::from_millis(100))=>{}}
            }
        };
        if let Err(error) = work.await {
            sender.send_replace(DataUpdate::Error(error));
        }
    };
    environment.jobs.spawn_on(work, &environment.runtime);
}
