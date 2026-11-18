//! Model-floor strategy over consistent actor reads. The generic mark actor owns refresh/cache policy.
use crate::{MarketDataHandle, error::DataError};
use market_gifts::valuation::{ModelPosition, Valuation, ValuationRequest};
use market_valuation::{Coverage, Evaluation, Strategy};
use std::collections::BTreeMap;
#[derive(Clone)]
pub struct ModelFloorStrategy {
    market: MarketDataHandle,
}
impl ModelFloorStrategy {
    pub fn new(market: MarketDataHandle) -> Self {
        Self { market }
    }
}
pub type ValuationHandle = market_valuation::Handle<ValuationRequest, Valuation, DataError>;
impl Strategy for ModelFloorStrategy {
    type Key = ValuationRequest;
    type Value = Valuation;
    type Error = DataError;
    fn normalize(&self, request: ValuationRequest) -> Result<ValuationRequest, DataError> {
        crate::query::valuation(&request)?;
        let mut grouped = BTreeMap::new();
        for p in request.positions {
            let quantity = grouped.entry(p.model).or_insert(0u32);
            *quantity = quantity
                .checked_add(p.quantity)
                .ok_or_else(|| DataError::invalid("portfolio quantity overflow"))?;
        }
        Ok(ValuationRequest {
            positions: grouped
                .into_iter()
                .map(|(model, quantity)| ModelPosition { model, quantity })
                .collect(),
            currency_id: request.currency_id,
        })
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.market.changes()
    }
    async fn evaluate(
        &self,
        request: ValuationRequest,
    ) -> Result<Evaluation<Valuation>, DataError> {
        let value = self.market.value_models(request).await?;
        Ok(Evaluation {
            revision: value.cursor.clone(),
            coverage: Coverage {
                priced: value.priced,
                unpriced: value.unpriced,
            },
            value,
        })
    }
}
