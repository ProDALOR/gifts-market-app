use crate::{InventoryError, InventoryPage, InventoryPageRequest};
use market_actors::error::DataError;
use market_gifts::valuation::{Valuation, ValuationRequest};
use std::future::Future;

/// Build one unsubmitted, replayable page operation. The actor submits it to the shared runtime.
/// Implementations declare shared rate keys and classify errors; they never queue or sleep.
pub trait InventoryProvider: Send + Sync + 'static {
    fn page(
        &self,
        request: InventoryPageRequest,
    ) -> impl market_invocations::Invocation<Output = InventoryPage, Error = InventoryError>;
}
pub trait ValuationProvider: Send + Sync + 'static {
    fn mark(
        &self,
        request: ValuationRequest,
    ) -> impl Future<Output = Result<std::sync::Arc<market_valuation::Mark<Valuation>>, DataError>> + Send;
}
impl ValuationProvider for market_actors::valuation::ValuationHandle {
    async fn mark(
        &self,
        request: ValuationRequest,
    ) -> Result<std::sync::Arc<market_valuation::Mark<Valuation>>, DataError> {
        self.mark(request).await
    }
}
