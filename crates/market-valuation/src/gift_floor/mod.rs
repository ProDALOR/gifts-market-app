//! One supervised gift strategy. Cached collection statistics are checked against each pinned view.
mod actor;
mod calculation;
mod display;
mod result;
mod statistics;
use crate::source::ValuationSource;
use crate::*;
use crate::{Handle, Mark, Service, ServiceError, Task, ValuationError};
pub use actor::GiftFloorActor;
pub use result::*;
use std::sync::Arc;
pub const METADATA: crate::StrategyMetadata = crate::StrategyMetadata {
    id: "gift_floor",
    name: "Gift floor",
    description: "A floor-based heuristic using collection and trait floors, capped by the matching model-and-backdrop floor.",
    documentation: include_str!("methodology.md"),
};
pub type GiftFloorMark = Mark<GiftFloorValuation>;
type NativeHandle = Handle<GiftValuationRequest, GiftFloorValuation, ValuationError>;
#[derive(Clone)]
pub struct ValuationHandle(NativeHandle);
impl ValuationHandle {
    pub fn metadata(&self) -> &'static crate::StrategyMetadata {
        crate::strategy(self.0.strategy()).expect("gift strategy was registered before startup")
    }
    pub async fn gift(
        &self,
        request: GiftValuationRequest,
    ) -> Result<Arc<GiftFloorMark>, ValuationError> {
        let currency = request.currency_id.clone();
        crate::client::invoke(
            &self.0,
            request,
            StrategyId::new(self.metadata().id).unwrap(),
            currency,
        )
        .await
    }
    pub async fn start(
        arbiter: &actix::ArbiterHandle,
        reads: market_replica::ReadExecutor,
    ) -> Result<(Self, Task), ServiceError> {
        let metadata = crate::strategy(METADATA.id).ok_or(ServiceError)?;
        StrategyId::new(metadata.id).map_err(|_| ServiceError)?;
        let service = Service::start_on(
            arbiter,
            metadata.id,
            move |life| GiftFloorActor::new(ValuationSource::new(reads), life),
            crate::Config {
                max_requests: 32,
                ..Default::default()
            },
        )
        .await?;
        Ok((Self(service.handle), service.task))
    }
}

#[cfg(test)]
mod tests;
