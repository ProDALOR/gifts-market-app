use super::execution::{ExecutionStatistics, MAX_OBSERVATIONS, eligible};
use super::*;
use crate::{
    gift_floor::statistics::BookCache,
    source::{BookScope, ValuationSnapshot},
};
use market_gifts::{GiftTarget, OrderSource};
use std::{cell::OnceCell, collections::BTreeSet};

/// A strategy owns its typed output. Actor admission, source reads and transport stay outside it.
pub trait Estimator {
    type Output;
    fn evaluate(&self, context: &ValuationContext<'_>) -> Result<Self::Output, ValuationError>;
}
pub struct ValuationContext<'a> {
    pub(super) snapshot: &'a ValuationSnapshot,
    pub(super) target: ResolvedTarget,
    pub(super) currency: CurrencyId,
    pub(super) period: Option<&'a market_gifts::Period>,
    books: &'a BookCache,
    executions: OnceCell<Result<ExecutionStatistics, ValuationError>>,
}
impl<'a> ValuationContext<'a> {
    pub(crate) fn new(
        snapshot: &'a ValuationSnapshot,
        request: &'a TargetValuationRequest,
        books: &'a BookCache,
    ) -> Result<Self, ValuationError> {
        snapshot.require_currency(&request.currency_id)?;
        Ok(Self {
            snapshot,
            target: request.target.resolve(snapshot)?,
            currency: request.currency_id.clone(),
            period: request.period.as_ref(),
            books,
            executions: OnceCell::new(),
        })
    }
    pub fn target(&self) -> &ResolvedTarget {
        &self.target
    }
    pub fn snapshot(&self) -> &ValuationSnapshot {
        self.snapshot
    }
    pub fn currency(&self) -> &CurrencyId {
        &self.currency
    }
    fn executions(&self) -> Result<&ExecutionStatistics, ValuationError> {
        if !self
            .period
            .is_some_and(|p| p.from.is_some() && p.until.is_some())
        {
            return Err(ValuationError::invalid(
                "execution strategies require an explicit period",
            ));
        }
        self.executions
            .get_or_init(|| ExecutionStatistics::read(self))
            .as_ref()
            .map_err(Clone::clone)
    }
    pub(crate) fn gift_floor(
        &self,
        lookup: market_gifts::GiftLookup,
    ) -> Result<GiftFloorValuation, ValuationError> {
        let gift = self.target.gift.clone().ok_or_else(|| {
            ValuationError::with_details(
                ValuationErrorCode::UnsupportedTarget,
                "gift_floor requires an individual gift",
                ValuationErrorDetails::strategy(StrategyId::new("gift_floor").unwrap()),
            )
        })?;
        Ok(self
            .books
            .get(
                self.snapshot,
                &BookScope {
                    collection: gift.collection_id.clone(),
                    currency: self.currency.clone(),
                },
            )?
            .report(gift, lookup, self.currency.clone()))
    }
}
pub struct AskFloor;
impl Estimator for AskFloor {
    type Output = AskFloorValuation;
    fn evaluate(&self, context: &ValuationContext<'_>) -> Result<Self::Output, ValuationError> {
        if context.target.gift.is_none() {
            let stats = context.books.get(
                context.snapshot,
                &BookScope {
                    collection: context.target.collection_id.clone(),
                    currency: context.currency.clone(),
                },
            )?;
            let (price, evidence) = stats.selection(
                context.target.model_id.as_ref(),
                context.target.backdrop_id.as_ref(),
            );
            return Ok(AskFloorValuation { price, evidence });
        }
        let query = context
            .target
            .query(context.currency.clone(), OrderSource::Asks, None);
        let mut token = String::new();
        let mut floor: Option<Amount> = None;
        let mut gifts = BTreeSet::new();
        let mut count = 0;
        let mut budget = market_replica::catalog_read::ScanBudget::default();
        loop {
            budget.page()?;
            let page = context.snapshot.valuation_orders(&query, token)?;
            for order in page.data {
                context.snapshot.check_cancelled()?;
                if order
                    .expires_at
                    .is_some_and(|expiry| expiry <= jiff::Timestamp::now())
                    || !eligible(&order, &context.target)
                {
                    continue;
                }
                count += 1;
                if count > MAX_OBSERVATIONS {
                    return Err(ValuationError::capacity("ask observation budget exceeded"));
                }
                if let GiftTarget::Gift { gift } = &order.target {
                    gifts.insert(gift.id.clone());
                }
                if floor
                    .as_ref()
                    .is_none_or(|old| order.price.decimal() < old.decimal())
                {
                    floor = Some(order.price.clone());
                }
            }
            if page.next_page_token.is_empty() {
                break;
            }
            token = page.next_page_token;
        }
        Ok(AskFloorValuation {
            price: floor.map_or_else(
                || {
                    unavailable(
                        ValuationIssueCode::NotEnoughData,
                        "No eligible asks match this target",
                        None,
                        None,
                    )
                },
                |value| ValuationMetric::Value { value },
            ),
            evidence: FloorEvidence {
                listings: count as u32,
                gifts: gifts.len() as u32,
            },
        })
    }
}
pub struct ExecutionMean;
impl Estimator for ExecutionMean {
    type Output = ExecutionPriceValuation;
    fn evaluate(&self, context: &ValuationContext<'_>) -> Result<Self::Output, ValuationError> {
        let stats = context.executions()?;
        Ok(ExecutionPriceValuation {
            price: stats.mean(),
            period: context.period.cloned().expect("validated period"),
            evidence: stats.evidence.clone(),
        })
    }
}
pub struct ExecutionMedian;
impl Estimator for ExecutionMedian {
    type Output = ExecutionPriceValuation;
    fn evaluate(&self, context: &ValuationContext<'_>) -> Result<Self::Output, ValuationError> {
        let stats = context.executions()?;
        Ok(ExecutionPriceValuation {
            price: stats.median(),
            period: context.period.cloned().expect("validated period"),
            evidence: stats.evidence.clone(),
        })
    }
}
