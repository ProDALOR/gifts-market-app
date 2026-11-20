//! One supervised gift strategy. Cached collection statistics are checked against each pinned view.
mod actor;
mod calculation;
mod display;
mod result;
pub(crate) mod statistics;
use crate::source::ValuationSource;
use crate::*;
use crate::{Handle, Mark, Service, ServiceError, ValuationError};
pub use actor::GiftFloorActor;
pub use result::*;
use std::sync::Arc;
pub const METADATA: crate::StrategyMetadata = crate::StrategyMetadata {
    id: "gift_floor",
    name: "Gift floor",
    description: "A floor-based heuristic using collection and trait floors, capped by the matching model-and-backdrop floor.",
    documentation: include_str!("methodology.md"),
    targets: &[crate::ValuationTargetKind::Gift],
    requires_period: false,
};
pub type GiftFloorMark = Mark<GiftFloorValuation>;
type NativeHandle = Handle<GiftValuationRequest, GiftFloorValuation, ValuationError>;
#[derive(Clone)]
pub struct ValuationHandle {
    gift: NativeHandle,
    targets: Handle<TargetValuationRequest, TargetValuationReport, ValuationError>,
}
impl ValuationHandle {
    pub fn gift_metadata(&self) -> &'static crate::StrategyMetadata {
        crate::strategy(self.gift.strategy()).expect("gift strategy was registered before startup")
    }
    pub async fn gift(
        &self,
        request: GiftValuationRequest,
    ) -> Result<Arc<GiftFloorMark>, ValuationError> {
        let currency = request.currency_id.clone();
        crate::client::invoke(
            &self.gift,
            request,
            StrategyId::new(self.gift_metadata().id).unwrap(),
            currency,
        )
        .await
    }
    pub async fn query(
        &self,
        request: TargetValuationRequest,
    ) -> Result<Arc<TargetValuationReport>, ValuationError> {
        request.validate()?;
        let mark = crate::client::invoke(
            &self.targets,
            request.clone(),
            StrategyId::new("valuation").unwrap(),
            request.currency_id,
        )
        .await?;
        Ok(Arc::new(mark.data.clone()))
    }
    pub async fn start(
        arbiter: &actix::ArbiterHandle,
        reads: market_replica::ReadExecutor,
    ) -> Result<(Self, ValuationTask), ServiceError> {
        let metadata = crate::strategy(METADATA.id).ok_or(ServiceError)?;
        StrategyId::new(metadata.id).map_err(|_| ServiceError)?;
        let cache = Arc::new(statistics::BookCache::default());
        let gift_cache = cache.clone();
        let gift_reads = reads.clone();
        let service = Service::start_on(
            arbiter,
            metadata.id,
            move |life| {
                GiftFloorActor::with_cache(ValuationSource::new(gift_reads), life, gift_cache)
            },
            crate::Config {
                max_requests: 32,
                ..Default::default()
            },
        )
        .await?;
        let targets = Service::start_on(
            arbiter,
            "valuation",
            move |life| {
                crate::market::MarketValuationActor::new(ValuationSource::new(reads), cache, life)
            },
            crate::Config {
                max_requests: 32,
                ..Default::default()
            },
        )
        .await?;
        Ok((
            Self {
                gift: service.handle,
                targets: targets.handle,
            },
            ValuationTask {
                gift: service.task,
                targets: targets.task,
            },
        ))
    }
}

#[cfg(test)]
pub(crate) mod tests;
