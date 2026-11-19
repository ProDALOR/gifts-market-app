//! Live slice and catalog computation; subscriber registration belongs to the actor.
use super::{DataUpdate, QueryFeed};
use crate::error::DataError;
use crate::{orderbooks::handle::OrderbooksHandle, runtime::ActorEnvironment};
use market_hub_protocol::wire as w;
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, watch};
use tokio_util::sync::CancellationToken;
use tonic::Status;
struct Completion {
    sender: watch::Sender<DataUpdate>,
    stop: CancellationToken,
    permit: Option<OwnedSemaphorePermit>,
}
impl Completion {
    fn new(feed: &QueryFeed, permit: OwnedSemaphorePermit) -> Self {
        Self {
            sender: feed.sender.clone(),
            stop: feed.stop.clone(),
            permit: Some(permit),
        }
    }
    fn finish(&mut self, error: DataError) {
        // A receiver can retry as soon as it sees the terminal update.
        drop(self.permit.take());
        self.stop.cancel();
        if !matches!(*self.sender.borrow(), DataUpdate::Error(_)) {
            self.sender.send_replace(DataUpdate::Error(error));
        }
    }
}
/// Coalesce data changes without retaining a producer after cancellation or receiver loss.
async fn wait_for_change(
    changed: &mut watch::Receiver<u64>,
    stop: &CancellationToken,
    sender: &watch::Sender<DataUpdate>,
) -> Result<bool, DataError> {
    tokio::select! { biased;
        _ = stop.cancelled() => return Ok(false),
        _ = sender.closed() => return Ok(false),
        result = changed.changed() => result.map_err(|_| Status::unavailable("Market updates stopped"))?,
    }
    tokio::select! { biased;
        _ = stop.cancelled() => Ok(false),
        _ = sender.closed() => Ok(false),
        _ = tokio::time::sleep(Duration::from_millis(100)) => Ok(true),
    }
}
impl Drop for Completion {
    fn drop(&mut self) {
        self.finish(Status::unavailable("Live subscription stopped").into());
    }
}
/// Catalog notifications use the replica's metadata clock, never order-price or freshness ticks.
pub(super) fn start_catalog(
    environment: &ActorEnvironment,
    feed: &QueryFeed,
    catalog: crate::catalog::handle::CatalogHandle,
    ids: Vec<market_gifts::CollectionId>,
    reader: market_replica::ReplicaReader,
    permit: OwnedSemaphorePermit,
) {
    let sender = feed.sender.clone();
    let stop = feed.stop.clone();
    let completion = Completion::new(feed, permit);
    let work = async move {
        let mut completion = completion;
        let mut changed = reader.changes();
        let work = async {
            let mut previous = None;
            let mut revision = None;
            loop {
                if stop.is_cancelled() || sender.is_closed() {
                    return Ok(());
                }
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
                if !wait_for_change(&mut changed, &stop, &sender).await? {
                    return Ok(());
                }
            }
        };
        if let Err(error) = work.await {
            completion.finish(error);
        }
    };
    environment.jobs.spawn_on(work, &environment.runtime);
}

pub(super) fn start_orders(
    environment: &ActorEnvironment,
    feed: &QueryFeed,
    orders: OrderbooksHandle,
    selection: super::types::OrdersFeed,
    reader: market_replica::ReplicaReader,
    permit: OwnedSemaphorePermit,
) {
    let super::types::OrdersFeed {
        request,
        slice,
        range: mut coverage,
    } = selection;
    let sender = feed.sender.clone();
    let stop = feed.stop.clone();
    let completion = Completion::new(feed, permit);
    let work = async move {
        let mut completion = completion;
        let mut changed = reader.changes();
        let work = async {
            let mut previous: Option<Arc<crate::OrderResult>> = None;
            let mut revision = None;
            loop {
                if stop.is_cancelled() || sender.is_closed() {
                    return Ok(());
                }
                changed.borrow_and_update();
                let current = reader.read_revision(&request)?;
                if revision != Some(current) {
                    let read = async {
                        if let Some(range) = &coverage {
                            orders.range_wait(request.clone(), range.clone()).await
                        } else {
                            orders.slice_wait(request.clone(), slice.clone()).await
                        }
                    };
                    let result = tokio::select! {biased;_=stop.cancelled()=>return Ok(()),_=sender.closed()=>return Ok(()),result=read=>result?};
                    let (next, current) = result;
                    let next = next.result;
                    coverage = Some(next.range.clone());
                    if previous.as_ref().is_none_or(|old| **old != next) {
                        previous = Some(Arc::new(next));
                    }
                    revision = Some(current);
                    sender.send_replace(DataUpdate::Orders {
                        version: current,
                        result: previous.as_ref().expect("range loaded").clone(),
                    });
                }
                if !wait_for_change(&mut changed, &stop, &sender).await? {
                    return Ok(());
                }
            }
        };
        if let Err(error) = work.await {
            completion.finish(error);
        }
    };
    environment.jobs.spawn_on(work, &environment.runtime);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn a_panicking_producer_retires_its_feed_and_releases_capacity() {
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = slots.clone().try_acquire_owned().unwrap();
        let (sender, updates) = watch::channel(DataUpdate::Connecting);
        let feed = QueryFeed {
            sender,
            stop: CancellationToken::new(),
        };
        let completion = Completion::new(&feed, permit);
        assert!(
            tokio::spawn(async move {
                let _completion = completion;
                panic!("simulated producer failure");
            })
            .await
            .unwrap_err()
            .is_panic()
        );
        assert!(!feed.reusable());
        assert!(matches!(*updates.borrow(), DataUpdate::Error(_)));
        assert!(slots.try_acquire_owned().is_ok());
    }
}
