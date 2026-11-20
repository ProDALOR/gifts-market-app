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
    async fn invoke(
        &self,
        work: Work,
        currency: market_gifts::CurrencyId,
    ) -> Result<Arc<Mark<Arc<crate::market::ValuationRead>>>, ValuationError> {
        crate::client::invoke(
            &self.native,
            work,
            StrategyId::new("valuation").unwrap(),
            currency,
        )
        .await
    }
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
        let currency = request.currency().clone();
        Ok(self
            .invoke(Work { request, previous }, currency)
            .await?
            .data
            .clone())
    }
    pub async fn watch(&self, request: ValuationQuery) -> Result<ValuationWatch, ValuationError> {
        let request = request.prepare()?;
        let permit = self
            .watch_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ValuationError::capacity("Valuation watch capacity reached"))?;
        // Listen before the initial admitted read, retaining changes during initialization.
        let changes = self.changes.clone();
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
                    _ = tokio::time::sleep_until(self.current.deadline) => {},
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
