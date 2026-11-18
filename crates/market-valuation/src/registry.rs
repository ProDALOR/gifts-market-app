use market_actors::error::{DataError, DataErrorCode};

use crate::{Handle, Mark};
use market_gifts::{CurrencyId, valuation::*};

use std::{collections::BTreeMap, sync::Arc};

pub type FloorStrategyHandle = Handle<PortfolioPositions, Valuation, DataError>;
pub type ModelMultiplierHandle = Handle<ModelMultiplierRequest, ModelMultiplier, DataError>;
pub type BackdropMultiplierHandle =
    Handle<BackdropMultiplierRequest, BackdropMultiplier, DataError>;
#[derive(Clone)]
pub enum RegisteredStrategy {
    Floor(FloorStrategyHandle),
    ModelMultiplier(ModelMultiplierHandle),
    BackdropMultiplier(BackdropMultiplierHandle),
}
impl From<FloorStrategyHandle> for RegisteredStrategy {
    fn from(h: FloorStrategyHandle) -> Self {
        Self::Floor(h)
    }
}
impl From<ModelMultiplierHandle> for RegisteredStrategy {
    fn from(h: ModelMultiplierHandle) -> Self {
        Self::ModelMultiplier(h)
    }
}
impl From<BackdropMultiplierHandle> for RegisteredStrategy {
    fn from(h: BackdropMultiplierHandle) -> Self {
        Self::BackdropMultiplier(h)
    }
}
impl RegisteredStrategy {
    fn id(&self) -> &'static str {
        match self {
            Self::Floor(h) => h.strategy(),
            Self::ModelMultiplier(h) => h.strategy(),
            Self::BackdropMultiplier(h) => h.strategy(),
        }
    }
}
#[derive(Debug, Clone)]
pub enum ValuationResult {
    ModelFloor(Arc<Mark<Valuation>>),
    CollectionFloor(Arc<Mark<Valuation>>),
    ModelMultiplier(Arc<Mark<ModelMultiplier>>),
    BackdropMultiplier(Arc<Mark<BackdropMultiplier>>),
}
#[derive(Clone)]
pub struct ValuationHandle {
    strategies: Arc<BTreeMap<StrategyId, RegisteredStrategy>>,
}
impl ValuationHandle {
    pub fn new<H: Into<RegisteredStrategy>>(
        handles: impl IntoIterator<Item = H>,
    ) -> Result<Self, DataError> {
        let mut strategies = BTreeMap::new();
        for handle in handles {
            let handle = handle.into();
            let id = StrategyId::new(handle.id()).map_err(|e| DataError::invalid(e.to_string()))?;
            let valid = match &handle {
                RegisteredStrategy::Floor(_) => id.is_portfolio_strategy(),
                RegisteredStrategy::ModelMultiplier(_) => id == StrategyId::model_multiplier(),
                RegisteredStrategy::BackdropMultiplier(_) => {
                    id == StrategyId::backdrop_multiplier()
                }
            };
            if !valid {
                return Err(DataError::invalid(
                    "strategy ID does not match its result contract",
                ));
            }
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
    fn unsupported(&self, strategy: StrategyId, currency: CurrencyId) -> DataError {
        let mut details = ValuationErrorDetails::strategy(strategy);
        details.currency_id = Some(currency);
        DataError::valuation(
            DataErrorCode::UnsupportedStrategy,
            format!(
                "Strategy is unavailable for this operation. Registered: {}",
                self.strategies
                    .keys()
                    .map(StrategyId::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            details,
        )
    }
    /// Portfolio estimators consume money-valued floor strategies explicitly.
    pub async fn mark(&self, request: ValuationRequest) -> Result<Arc<Mark<Valuation>>, DataError> {
        let id = request.strategy.clone();
        let currency = request.currency_id.clone();
        let Some(RegisteredStrategy::Floor(handle)) = self.strategies.get(&id) else {
            return Err(self.unsupported(id, currency));
        };
        invoke(
            handle,
            PortfolioPositions {
                positions: request.positions,
                currency_id: request.currency_id,
            },
            id,
            currency,
        )
        .await
    }
    pub async fn query(&self, request: ValuationQuery) -> Result<ValuationResult, DataError> {
        match request {
            ValuationQuery::ModelFloor(p) => self
                .mark(ValuationRequest {
                    strategy: StrategyId::model_floor(),
                    positions: p.positions,
                    currency_id: p.currency_id,
                })
                .await
                .map(ValuationResult::ModelFloor),
            ValuationQuery::CollectionFloor(p) => self
                .mark(ValuationRequest {
                    strategy: StrategyId::collection_floor(),
                    positions: p.positions,
                    currency_id: p.currency_id,
                })
                .await
                .map(ValuationResult::CollectionFloor),
            ValuationQuery::ModelMultiplier(p) => {
                let id = StrategyId::model_multiplier();
                let currency = p.currency_id.clone();
                let Some(RegisteredStrategy::ModelMultiplier(h)) = self.strategies.get(&id) else {
                    return Err(self.unsupported(id, currency));
                };
                invoke(h, p, id, currency)
                    .await
                    .map(ValuationResult::ModelMultiplier)
            }
            ValuationQuery::BackdropMultiplier(p) => {
                let id = StrategyId::backdrop_multiplier();
                let currency = p.currency_id.clone();
                let Some(RegisteredStrategy::BackdropMultiplier(h)) = self.strategies.get(&id)
                else {
                    return Err(self.unsupported(id, currency));
                };
                invoke(h, p, id, currency)
                    .await
                    .map(ValuationResult::BackdropMultiplier)
            }
        }
    }
}
async fn invoke<K: Send + 'static, V: Send + Sync + 'static>(
    handle: &Handle<K, V, DataError>,
    request: K,
    strategy: StrategyId,
    currency: CurrencyId,
) -> Result<Arc<Mark<V>>, DataError> {
    let mark = handle.evaluate(request).await.map_err(|error| {
        let error = match error {
            crate::Error::Strategy(e) => e,
            crate::Error::Busy => DataError::capacity("Valuation requests are busy"),
            crate::Error::Stopped => DataError::unavailable("Valuation service is unavailable"),
            crate::Error::TimedOut => DataError::timed_out("Valuation request timed out"),
        };
        error.with_valuation(strategy.clone(), currency.clone())
    })?;
    if mark.strategy != strategy.as_str() {
        return Err(
            DataError::unavailable("Valuation strategy returned an inconsistent mark")
                .with_valuation(strategy, currency),
        );
    }
    Ok(mark)
}
