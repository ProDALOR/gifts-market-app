use crate::{source::ValuationSource, values::ValuesRead, *};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
pub(crate) enum Work {
    Query(TargetValuationRequest),
    Values {
        request: MarketValuesRequest,
        previous: Option<Arc<ValuesRead>>,
    },
}
pub(crate) enum Output {
    Query(Arc<TargetValuationReport>),
    Values(Arc<ValuesRead>),
}
#[derive(Clone)]
pub struct ValuationHandle {
    native: Handle<Work, Output, ValuationError>,
    changes: watch::Receiver<u64>,
    watch_slots: Arc<tokio::sync::Semaphore>,
}
impl ValuationHandle {
    pub fn gift_metadata(&self) -> &'static StrategyMetadata {
        crate::strategy("gift_floor").unwrap()
    }
    pub async fn gift(
        &self,
        request: GiftValuationRequest,
    ) -> Result<Arc<GiftFloorMark>, ValuationError> {
        let currency = request.currency_id.clone();
        let input = TargetValuationRequest {
            target: ValuationTarget::Gift { gift: request.gift },
            currency_id: currency.clone(),
            strategies: vec![StrategyId::new("gift_floor").unwrap()],
            period: None,
        };
        let mark = self.invoke(Work::Query(input), currency).await?;
        let Output::Query(report) = &mark.data else {
            return Err(ValuationError::unavailable(
                "Unexpected gift valuation output",
            ));
        };
        match &report.results[0] {
            StrategyOutcome::Value(StrategyValuation::GiftFloor(data)) => Ok(Arc::new(Mark {
                strategy: "gift_floor",
                data: data.clone(),
                revision: mark.revision.clone(),
                evaluated_at: report.evaluated_at,
                stale: report.stale,
            })),
            StrategyOutcome::Error { error, .. } => Err(ValuationError::with_details(
                error.code,
                error.message.clone(),
                ValuationErrorDetails::strategy(StrategyId::new("gift_floor").unwrap()),
            )),
            _ => Err(ValuationError::unavailable(
                "Gift strategy returned an inconsistent result",
            )),
        }
    }
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
        request: TargetValuationRequest,
    ) -> Result<Arc<TargetValuationReport>, ValuationError> {
        request.validate()?;
        let currency = request.currency_id.clone();
        let result = self.invoke(Work::Query(request), currency).await?;
        match &result.data {
            Output::Query(report) => Ok(report.clone()),
            _ => Err(ValuationError::unavailable("Unexpected valuation output")),
        }
    }
    async fn values_read(
        &self,
        request: MarketValuesRequest,
        previous: Option<Arc<ValuesRead>>,
    ) -> Result<Arc<ValuesRead>, ValuationError> {
        request.validate()?;
        let currency = request.currency_id.clone();
        let result = self
            .invoke(Work::Values { request, previous }, currency)
            .await?;
        match &result.data {
            Output::Values(values) => Ok(values.clone()),
            _ => Err(ValuationError::unavailable(
                "Unexpected market values output",
            )),
        }
    }
    pub async fn values(
        &self,
        request: MarketValuesRequest,
    ) -> Result<Arc<MarketValues>, ValuationError> {
        Ok(self.values_read(request, None).await?.values.clone())
    }
    pub async fn watch_values(
        &self,
        request: MarketValuesRequest,
    ) -> Result<MarketValuesWatch, ValuationError> {
        let permit = self
            .watch_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ValuationError::capacity("Market values watch capacity reached"))?;
        let changes = self.changes.clone();
        let current = self.values_read(request.clone(), None).await?;
        Ok(MarketValuesWatch {
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
pub struct MarketValuesWatch {
    handle: ValuationHandle,
    request: MarketValuesRequest,
    current: Arc<ValuesRead>,
    changes: watch::Receiver<u64>,
    dirty: bool,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl MarketValuesWatch {
    pub fn current(&self) -> Arc<MarketValues> {
        self.current.values.clone()
    }
    pub async fn changed(&mut self) -> Result<Arc<MarketValues>, ValuationError> {
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
                .values_read(self.request.clone(), Some(self.current.clone()))
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
