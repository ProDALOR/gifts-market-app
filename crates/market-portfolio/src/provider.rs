use crate::*;
use std::{future::Future, sync::Arc};
pub trait InventoryProvider: Send + Sync + 'static {
    fn page(
        &self,
        request: InventoryPageRequest,
    ) -> impl market_invocations::Invocation<Output = InventoryPage, Error = InventoryError>;
}
pub trait ValuationProvider: Send + Sync + 'static {
    /// Publish an increasing processed-gift count as work advances. The actor
    /// cancels a stalled future; progress is not a heartbeat or a partial report.
    fn value(
        &self,
        gifts: Arc<Vec<InventoryGift>>,
        currency: market_gifts::CurrencyId,
        progress: tokio::sync::watch::Sender<usize>,
    ) -> impl Future<Output = Result<Arc<InventoryValuation>, market_valuation::ValuationError>> + Send;
}
impl ValuationProvider for market_valuation::ValuationHandle {
    async fn value(
        &self,
        gifts: Arc<Vec<InventoryGift>>,
        currency: market_gifts::CurrencyId,
        progress: tokio::sync::watch::Sender<usize>,
    ) -> Result<Arc<InventoryValuation>, market_valuation::ValuationError> {
        self.inventory(gifts, currency, progress).await
    }
}
