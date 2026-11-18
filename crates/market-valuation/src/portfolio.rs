//! Explicit quantity-based portfolio floors, independent of the gift strategy.
use crate::{ValuationError, ValuationErrorCode, client::invoke};

use crate::*;
use crate::{Handle, Mark};
use market_gifts::CurrencyId;

use std::{collections::BTreeMap, sync::Arc};

pub type FloorStrategyHandle = Handle<PortfolioPositions, Valuation, ValuationError>;
#[derive(Clone)]
pub struct PortfolioValuationHandle {
    strategies: Arc<BTreeMap<StrategyId, FloorStrategyHandle>>,
}
impl PortfolioValuationHandle {
    pub fn new(
        handles: impl IntoIterator<Item = FloorStrategyHandle>,
    ) -> Result<Self, ValuationError> {
        let mut strategies = BTreeMap::new();
        for handle in handles {
            let id = StrategyId::new(handle.strategy())
                .map_err(|e| ValuationError::invalid(e.to_string()))?;
            let valid = id.is_portfolio_strategy();
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
        })
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
        let Some(handle) = self.strategies.get(&id) else {
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
}
