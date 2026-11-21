use crate::{source::ValuationSource, *};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
pub(crate) struct Work {
    pub request: crate::market::PreparedQuery,
    pub previous: Option<Arc<crate::market::ValuationRead>>,
}
#[derive(Clone)]
pub struct ValuationHandle {
    native: Handle<Work, Arc<crate::market::ValuationRead>, ValuationError>,
    changes: watch::Receiver<u64>,
    watch_slots: Arc<tokio::sync::Semaphore>,
}
impl ValuationHandle {
    pub async fn query(
        &self,
        request: ValuationQuery,
    ) -> Result<Arc<ValuationReport>, ValuationError> {
        Ok(self.read(request.prepare()?, None).await?.report.clone())
    }
    async fn read(
        &self,
        request: crate::market::PreparedQuery,
        previous: Option<Arc<crate::market::ValuationRead>>,
    ) -> Result<Arc<crate::market::ValuationRead>, ValuationError> {
        let mark = self
            .native
            .evaluate(Work { request, previous })
            .await
            .map_err(crate::client::actor_error)?;
        Ok(mark.data.clone())
    }

    pub async fn watch(&self, request: ValuationQuery) -> Result<ValuationWatch, ValuationError> {
        let request = request.prepare()?;
        let permit = self
            .watch_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ValuationError::capacity("Valuation watch capacity reached"))?;
        // Listen before the initial admitted read, retaining changes during initialization.
        let mut changes = self.changes.clone();
        changes.borrow_and_update();
        let current = self.read(request.clone(), None).await?;
        Ok(ValuationWatch {
            handle: self.clone(),
            request,
            current,
            changes,
            dirty: false,
            _permit: permit,
        })
    }
    pub async fn start(
        arbiter: &actix::ArbiterHandle,
        reads: market_replica::ReadExecutor,
    ) -> Result<(Self, ValuationTask), ServiceError> {
        let changes = reads.reader().changes();
        let service = Service::start_on(
            arbiter,
            "valuation",
            move |life| {
                crate::market::MarketValuationActor::new(
                    ValuationSource::new(reads),
                    Arc::new(crate::floors::BookCache::default()),
                    life,
                )
            },
            Config {
                max_requests: 32,
                ..Default::default()
            },
        )
        .await?;
        Ok((
            Self {
                native: service.handle,
                changes,
                watch_slots: Arc::new(tokio::sync::Semaphore::new(32)),
            },
            service.task,
        ))
    }
}
/// Pull-based watch: dropping it or its pending next() releases all work through normal cancellation.
pub struct ValuationWatch {
    handle: ValuationHandle,
    request: crate::market::PreparedQuery,
    current: Arc<crate::market::ValuationRead>,
    changes: watch::Receiver<u64>,
    dirty: bool,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl ValuationWatch {
    pub fn current(&self) -> Arc<ValuationReport> {
        self.current.report.clone()
    }
    pub async fn changed(&mut self) -> Result<Arc<ValuationReport>, ValuationError> {
        loop {
            if !self.dirty {
                tokio::select! {
                    _ = self.handle.native.closed() => return Err(ValuationError::unavailable("Valuation service stopped")),
                    _ = tokio::time::sleep_until(self.current.deadline()) => {},
                    changed = self.changes.changed() => changed.map_err(|_|ValuationError::unavailable("Valuation source closed"))?,
                }
                // Retain this across cancellation of changed(), including its debounce/read.
                self.dirty = true;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {},
                    _ = self.handle.native.closed() => return Err(ValuationError::unavailable("Valuation service stopped")),
                }
            }
            let next = self
                .handle
                .read(self.request.clone(), Some(self.current.clone()))
                .await?;
            self.dirty = false;
            if Arc::ptr_eq(&next, &self.current) {
                continue;
            }
            self.current = next;
            return Ok(self.current());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        gift_floor::tests::{ask, book},
        test_support::MarketDataService,
    };
    use market_hub_protocol::wire as w;
    #[tokio::test]
    async fn watch_baseline_ignores_past_changes_but_retains_updates_during_setup() {
        let (_dir, mut writer) = book();
        let data = MarketDataService::start(writer.reader(), 2, 1)
            .await
            .unwrap();
        let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
            .await
            .unwrap();
        let query = ValuationQuery {
            target: ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: Default::default(),
            },
            currency_id: "GRAM".into(),
            strategies: vec![StrategyId::new("ask_floor").unwrap()],
            period: None,
        };
        writer
            .apply(w::Batch {
                cursor: "before-watch".into(),
                changes: vec![ask("before", "m", "b", "2", 999, None)],
            })
            .unwrap();
        let watch = handle.watch(query.clone()).await.unwrap();
        assert!(
            !watch.changes.has_changed().unwrap(),
            "Historical notifications do not trigger another read"
        );
        drop(watch);
        let executor = data.reader.executor();
        let held = executor.budget().wait().await.unwrap();
        let held2 = executor.budget().wait().await.unwrap();
        let mut setup = Box::pin(handle.watch(query));
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut setup)
                .await
                .is_err()
        );
        writer
            .apply(w::Batch {
                cursor: "during-watch".into(),
                changes: vec![ask("during", "m", "b", "1", 998, None)],
            })
            .unwrap();
        drop(held);
        drop(held2);
        let watch = setup.await.unwrap();
        assert!(
            watch.changes.has_changed().unwrap(),
            "Updates during initialization remain observable"
        );
        let StrategyOutcome::Value(StrategyValuation::AskFloor(value)) =
            &watch.current.report.results[0]
        else {
            panic!()
        };
        assert_eq!(value.price.value().unwrap().as_str(), "1");
        drop(watch);
        task.shutdown().await.unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
}
