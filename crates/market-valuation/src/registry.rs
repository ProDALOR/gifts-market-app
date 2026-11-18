use crate::{ValuationError, ValuationErrorCode};

use crate::{Handle, Mark};
use market_gifts::{CurrencyId, valuation::*};

use std::{collections::BTreeMap, sync::Arc};

pub type FloorStrategyHandle = Handle<PortfolioPositions, Valuation, ValuationError>;
pub type ModelMultiplierHandle = Handle<ModelMultiplierRequest, ModelMultiplier, ValuationError>;
pub type BackdropMultiplierHandle =
    Handle<BackdropMultiplierRequest, BackdropMultiplier, ValuationError>;
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
    pub(crate) reader: Option<market_actors::MarketReader>,
    pub(crate) gift_slots: Arc<tokio::sync::Semaphore>,
}
impl ValuationHandle {
    pub fn new<H: Into<RegisteredStrategy>>(
        handles: impl IntoIterator<Item = H>,
    ) -> Result<Self, ValuationError> {
        let mut strategies = BTreeMap::new();
        for handle in handles {
            let handle = handle.into();
            let id =
                StrategyId::new(handle.id()).map_err(|e| ValuationError::invalid(e.to_string()))?;
            let valid = match &handle {
                RegisteredStrategy::Floor(_) => id.is_portfolio_strategy(),
                RegisteredStrategy::ModelMultiplier(_) => id == StrategyId::model_multiplier(),
                RegisteredStrategy::BackdropMultiplier(_) => {
                    id == StrategyId::backdrop_multiplier()
                }
            };
            if !valid {
                return Err(ValuationError::invalid(
                    "strategy ID does not match its result contract",
                ));
            }
            if strategies.insert(id, handle).is_some() {
                return Err(ValuationError::invalid(
                    "valuation strategy is already registered",
                ));
            }
        }
        if strategies.is_empty() {
            return Err(ValuationError::invalid(
                "register at least one valuation strategy",
            ));
        }
        Ok(Self {
            strategies: Arc::new(strategies),
            reader: None,
            gift_slots: Arc::new(tokio::sync::Semaphore::new(32)),
        })
    }
    pub fn with_reader(mut self, reader: market_actors::MarketReader) -> Self {
        self.reader = Some(reader);
        self
    }
    pub fn strategies(&self) -> impl Iterator<Item = &StrategyId> {
        self.strategies.keys()
    }
    fn unsupported(&self, strategy: StrategyId, currency: CurrencyId) -> ValuationError {
        let mut details = ValuationErrorDetails::strategy(strategy);
        details.currency_id = Some(currency);
        ValuationError::with_details(
            ValuationErrorCode::UnsupportedStrategy,
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
    pub async fn mark(
        &self,
        request: ValuationRequest,
    ) -> Result<Arc<Mark<Valuation>>, ValuationError> {
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
    pub async fn query(&self, request: ValuationQuery) -> Result<ValuationResult, ValuationError> {
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
    handle: &Handle<K, V, ValuationError>,
    request: K,
    strategy: StrategyId,
    currency: CurrencyId,
) -> Result<Arc<Mark<V>>, ValuationError> {
    let mark = handle.evaluate(request).await.map_err(|error| {
        let error = match error {
            crate::Error::Strategy(e) => e,
            crate::Error::Busy => ValuationError::capacity("Valuation requests are busy"),
            crate::Error::Stopped => {
                ValuationError::unavailable("Valuation service is unavailable")
            }
            crate::Error::TimedOut => ValuationError::timed_out("Valuation request timed out"),
        };
        error.with_context(strategy.clone(), currency.clone())
    })?;
    if mark.strategy != strategy.as_str() {
        return Err(ValuationError::unavailable(
            "Valuation strategy returned an inconsistent mark",
        )
        .with_context(strategy, currency));
    }
    Ok(mark)
}
