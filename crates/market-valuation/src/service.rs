use crate::{floor_projection::FloorRead, source::ValuationSource, *};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
pub(crate) enum Work {
    Query(ValuationQuery),
    Floors {
        request: FloorQuery,
        previous: Option<Arc<FloorRead>>,
    },
}
pub(crate) enum Output {
    Query(Arc<ValuationReport>),
    Floors(Arc<FloorRead>),
}
#[derive(Clone)]
pub struct ValuationHandle {
    native: Handle<Work, Output, ValuationError>,
    changes: watch::Receiver<u64>,
    watch_slots: Arc<tokio::sync::Semaphore>,
}
impl ValuationHandle {
    async fn invoke(
        &self,
        work: Work,
        currency: market_gifts::CurrencyId,
    ) -> Result<Arc<Mark<Output>>, ValuationError> {
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
        request.validate()?;
        let currency = request.currency_id.clone();
        let result = self.invoke(Work::Query(request), currency).await?;
        match &result.data {
            Output::Query(report) => Ok(report.clone()),
            _ => Err(ValuationError::unavailable("Unexpected valuation output")),
        }
    }
    async fn floor_read(
        &self,
        request: FloorQuery,
        previous: Option<Arc<FloorRead>>,
    ) -> Result<Arc<FloorRead>, ValuationError> {
        request.validate()?;
        let currency = request.currency_id.clone();
        let result = self
            .invoke(Work::Floors { request, previous }, currency)
            .await?;
        match &result.data {
            Output::Floors(values) => Ok(values.clone()),
            _ => Err(ValuationError::unavailable(
                "Unexpected market values output",
            )),
        }
    }
    pub async fn floors(&self, request: FloorQuery) -> Result<Arc<FloorReport>, ValuationError> {
        Ok(self.floor_read(request, None).await?.values.clone())
    }
    pub async fn watch_floors(&self, request: FloorQuery) -> Result<FloorWatch, ValuationError> {
        let permit = self
            .watch_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ValuationError::capacity("Market values watch capacity reached"))?;
        let changes = self.changes.clone();
        let current = self.floor_read(request.clone(), None).await?;
        Ok(FloorWatch {
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
pub struct FloorWatch {
    handle: ValuationHandle,
    request: FloorQuery,
    current: Arc<FloorRead>,
    changes: watch::Receiver<u64>,
    dirty: bool,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl FloorWatch {
    pub fn current(&self) -> Arc<FloorReport> {
        self.current.values.clone()
    }
    pub async fn changed(&mut self) -> Result<Arc<FloorReport>, ValuationError> {
        loop {
            if !self.dirty {
                tokio::select! {
                    _ = self.handle.native.closed() => return Err(ValuationError::unavailable("Market values service stopped")),
                    _ = tokio::time::sleep_until(self.current.deadline) => {},
                    changed = self.changes.changed() => changed.map_err(|_|ValuationError::unavailable("Market values source closed"))?,
                }
                // Retain this across cancellation of changed(), including its debounce/read.
                self.dirty = true;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {},
                    _ = self.handle.native.closed() => return Err(ValuationError::unavailable("Market values service stopped")),
                }
            }
            let next = self
                .handle
                .floor_read(self.request.clone(), Some(self.current.clone()))
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
