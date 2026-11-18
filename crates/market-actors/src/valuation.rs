//! Named valuation strategies over consistent actor reads, sharing one market-read budget.
use crate::{
    MarketDataHandle,
    error::{DataError, DataErrorCode},
};
use market_gifts::valuation::{
    ModelPosition, StrategyId, Valuation, ValuationErrorDetails, ValuationRequest,
};
use market_valuation::{Coverage, Evaluation, Strategy};
use std::{collections::BTreeMap, sync::Arc};
pub type StrategyHandle = market_valuation::Handle<ValuationRequest, Valuation, DataError>;
/// Immutable registry assembled by the application. No implicit strategy fallback.
#[derive(Clone)]
pub struct ValuationHandle {
    strategies: Arc<BTreeMap<StrategyId, StrategyHandle>>,
}
impl ValuationHandle {
    pub fn new(handles: impl IntoIterator<Item = StrategyHandle>) -> Result<Self, DataError> {
        let mut strategies = BTreeMap::new();
        for handle in handles {
            let id = StrategyId::new(handle.strategy())
                .map_err(|e| DataError::invalid(e.to_string()))?;
            if strategies.insert(id, handle).is_some() {
                return Err(DataError::invalid(
                    "valuation strategy is already registered",
                ));
            }
        }
        if strategies.is_empty() {
            return Err(DataError::invalid(
                "register at least one valuation strategy",
            ));
        }
        Ok(Self {
            strategies: Arc::new(strategies),
        })
    }
    pub fn strategies(&self) -> impl Iterator<Item = &StrategyId> {
        self.strategies.keys()
    }
    pub async fn mark(
        &self,
        request: ValuationRequest,
    ) -> Result<Arc<market_valuation::Mark<Valuation>>, DataError> {
        let strategy = request.strategy.clone();
        let currency = request.currency_id.clone();
        let Some(handle) = self.strategies.get(&strategy) else {
            let mut details = ValuationErrorDetails::strategy(strategy);
            details.currency_id = Some(currency);
            return Err(DataError::valuation(
                DataErrorCode::UnsupportedStrategy,
                format!(
                    "Unknown valuation strategy. Available: {}",
                    self.strategies
                        .keys()
                        .map(StrategyId::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                details,
            ));
        };
        let mark = handle.mark(request).await.map_err(|error| {
            let error = match error {
                market_valuation::Error::Strategy(error) => error,
                market_valuation::Error::Busy => DataError::capacity("Valuation requests are busy"),
                market_valuation::Error::Stopped => {
                    DataError::unavailable("Valuation service is unavailable")
                }
                market_valuation::Error::TimedOut => {
                    DataError::timed_out("Valuation request timed out")
                }
            };
            error.with_valuation(strategy.clone(), currency)
        })?;
        if mark.strategy != strategy.as_str() {
            return Err(
                DataError::unavailable("Valuation strategy returned an inconsistent mark")
                    .with_valuation(strategy, mark.currency_id.clone()),
            );
        }
        Ok(mark)
    }
}
fn normalize(request: ValuationRequest, id: &str) -> Result<ValuationRequest, DataError> {
    crate::query::valuation(&request)?;
    if request.strategy.as_str() != id {
        return Err(DataError::invalid(
            "valuation request targets another strategy",
        ));
    }
    let mut grouped = BTreeMap::new();
    for p in request.positions {
        let quantity = grouped.entry(p.model).or_insert(0u32);
        *quantity = quantity
            .checked_add(p.quantity)
            .ok_or_else(|| DataError::invalid("portfolio quantity overflow"))?;
    }
    Ok(ValuationRequest {
        strategy: request.strategy,
        positions: grouped
            .into_iter()
            .map(|(model, quantity)| ModelPosition { model, quantity })
            .collect(),
        currency_id: request.currency_id,
    })
}
async fn evaluate(
    market: &MarketDataHandle,
    request: ValuationRequest,
) -> Result<Evaluation<Valuation>, DataError> {
    let strategy = request.strategy.clone();
    let value = market.valuation_wait(request).await?;
    if value.priced == 0 && value.unpriced > 0 {
        return Err(DataError::valuation(
            DataErrorCode::NotEnoughData,
            "No requested positions could be valued with this strategy and currency",
            ValuationErrorDetails {
                strategy,
                currency_id: Some(value.currency_id),
                revision: Some(value.cursor),
                required_samples: Some(1),
                available_samples: Some(0),
                unpriced: value.rows,
            },
        ));
    }
    Ok(Evaluation {
        revision: value.cursor.clone(),
        coverage: Coverage {
            priced: value.priced,
            unpriced: value.unpriced,
        },
        value,
    })
}
#[derive(Clone)]
pub struct ModelFloorStrategy {
    market: MarketDataHandle,
}
impl ModelFloorStrategy {
    pub fn new(market: MarketDataHandle) -> Self {
        Self { market }
    }
}
impl Strategy for ModelFloorStrategy {
    const ID: &'static str = "model_floor";
    type Key = ValuationRequest;
    type Value = Valuation;
    type Error = DataError;
    fn normalize(&self, request: ValuationRequest) -> Result<ValuationRequest, DataError> {
        normalize(request, Self::ID)
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.market.changes()
    }
    async fn evaluate(
        &self,
        request: ValuationRequest,
    ) -> Result<Evaluation<Valuation>, DataError> {
        evaluate(&self.market, request).await
    }
}
#[derive(Clone)]
pub struct CollectionFloorStrategy {
    market: MarketDataHandle,
}
impl CollectionFloorStrategy {
    pub fn new(market: MarketDataHandle) -> Self {
        Self { market }
    }
}
impl Strategy for CollectionFloorStrategy {
    const ID: &'static str = "collection_floor";
    type Key = ValuationRequest;
    type Value = Valuation;
    type Error = DataError;
    fn normalize(&self, request: ValuationRequest) -> Result<ValuationRequest, DataError> {
        normalize(request, Self::ID)
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.market.changes()
    }
    async fn evaluate(
        &self,
        request: ValuationRequest,
    ) -> Result<Evaluation<Valuation>, DataError> {
        evaluate(&self.market, request).await
    }
}
