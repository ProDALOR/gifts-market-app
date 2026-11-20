use super::*;
use market_gifts::{GiftOrder, GiftSort, GiftTarget, OrderSource};
use rust_decimal::Decimal;
use std::collections::BTreeSet;
pub(super) const MAX_OBSERVATIONS: usize = 50_000;
const MINIMUM: u32 = 3;
pub(super) struct ExecutionStatistics {
    prices: Vec<Amount>,
    pub evidence: ExecutionEvidence,
}
impl ExecutionStatistics {
    pub fn read(context: &ValuationContext<'_>) -> Result<Self, ValuationError> {
        let mut result = Self {
            prices: Vec::new(),
            evidence: ExecutionEvidence {
                executions: 0,
                gifts: 0,
                venues: 0,
                minimum_executions: MINIMUM,
                minimum_gifts: if context.target.gift().is_some() {
                    1
                } else {
                    MINIMUM
                },
                first_execution: None,
                last_execution: None,
            },
        };
        let mut gifts = BTreeSet::new();
        let mut venues = BTreeSet::new();
        let mut query = context.target.query(
            context.currency.clone(),
            OrderSource::History,
            context.period.cloned(),
        );
        query.sort = Some(GiftSort::TimeDesc);
        let mut token = String::new();
        let mut scanned = 0;
        let mut budget = market_replica::catalog_read::ScanBudget::default();
        loop {
            budget.page()?;
            let page = context.snapshot.valuation_orders(&query, token)?;
            scanned += page.data.len();
            if scanned > MAX_OBSERVATIONS {
                return Err(ValuationError::capacity(
                    "execution observation budget exceeded; shorten the period",
                ));
            }
            for order in page.data {
                context.snapshot.check_cancelled()?;
                if !eligible(&order, &context.target) {
                    continue;
                }
                result.prices.push(order.price.clone());
                if let GiftTarget::Gift { gift } = &order.target {
                    gifts.insert(gift.id.clone());
                }
                venues.insert(order.book.venue_id.clone());
                let time = order.executed_at.expect("checked execution time");
                result.evidence.first_execution = Some(
                    result
                        .evidence
                        .first_execution
                        .map_or(time, |old| old.min(time)),
                );
                result.evidence.last_execution = Some(
                    result
                        .evidence
                        .last_execution
                        .map_or(time, |old| old.max(time)),
                );
            }
            if page.next_page_token.is_empty() {
                break;
            }
            token = page.next_page_token;
        }
        result.prices.sort_unstable_by_key(Amount::decimal);
        result.evidence.executions = result.prices.len() as u32;
        result.evidence.gifts = gifts.len() as u32;
        result.evidence.venues = venues.len() as u32;
        Ok(result)
    }
    fn calculate(&self, f: impl FnOnce(&[Amount]) -> Option<Decimal>) -> ValuationMetric {
        let (count, required) = if self.evidence.executions < MINIMUM {
            (self.evidence.executions, MINIMUM)
        } else {
            (self.evidence.gifts, self.evidence.minimum_gifts)
        };
        if count < required {
            return unavailable(
                ValuationIssueCode::NotEnoughData,
                "Not enough observed executions or distinct identifiable gifts for this target",
                Some(required),
                Some(count),
            );
        }
        match f(&self.prices).and_then(|v| Amount::try_from(v).ok()) {
            Some(value) => ValuationMetric::Value { value },
            None => unavailable(
                ValuationIssueCode::CalculationOverflow,
                "Execution price cannot be represented safely",
                None,
                None,
            ),
        }
    }
    pub fn mean(&self) -> ValuationMetric {
        // Normalize by the largest positive price before summing. An arithmetic
        // mean cannot exceed that maximum, even when the raw sum overflows Decimal.
        self.calculate(|prices| {
            if let Some(sum) = prices
                .iter()
                .try_fold(Decimal::ZERO, |sum, p| sum.checked_add(p.decimal()))
            {
                return sum.checked_div(Decimal::from(prices.len() as u64));
            }
            let max = prices.last()?.decimal();
            let scaled = prices.iter().try_fold(Decimal::ZERO, |sum, p| {
                sum.checked_add(p.decimal().checked_div(max)?)
            })?;
            scaled
                .checked_div(Decimal::from(prices.len() as u64))?
                .checked_mul(max)
        })
    }
    pub fn median(&self) -> ValuationMetric {
        self.calculate(|prices| {
            let middle = prices.len() / 2;
            let upper = prices.get(middle)?.decimal();
            if prices.len() % 2 == 1 {
                Some(upper)
            } else {
                let lower = prices[middle - 1].decimal();
                lower.checked_add(upper.checked_sub(lower)?.checked_div(Decimal::TWO)?)
            }
        })
    }
}
pub(super) fn eligible(order: &GiftOrder, target: &ResolvedTarget) -> bool {
    if order.price.decimal() <= Decimal::ZERO || order.quantity == 0 {
        return false;
    }
    if let Some(wanted) = target.gift() {
        return matches!(&order.target,GiftTarget::Gift{gift} if gift.id==wanted.id);
    }
    match &order.target {
        GiftTarget::Gift { gift } => {
            target
                .model()
                .is_none_or(|id| gift.traits.model.as_ref() == Some(id))
                && target
                    .backdrop()
                    .is_none_or(|id| gift.traits.backdrop.as_ref() == Some(id))
        }
        GiftTarget::Criteria { criteria } => {
            target
                .model()
                .is_none_or(|id| criteria.models.as_slice() == [id.clone()])
                && target
                    .backdrop()
                    .is_none_or(|id| criteria.backdrops.as_slice() == [id.clone()])
        }
        GiftTarget::Unsupported { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stats(prices: &[&str]) -> ExecutionStatistics {
        let mut prices: Vec<_> = prices
            .iter()
            .map(|price| Amount::new(*price).unwrap())
            .collect();
        prices.sort_unstable_by_key(Amount::decimal);
        ExecutionStatistics {
            evidence: ExecutionEvidence {
                executions: prices.len() as u32,
                gifts: prices.len() as u32,
                venues: 1,
                minimum_executions: 3,
                minimum_gifts: 3,
                first_execution: None,
                last_execution: None,
            },
            prices,
        }
    }
    #[test]
    fn execution_prices_preserve_decimal_precision_and_even_medians_without_sum_overflow() {
        let sample = stats(&["0.1", "0.2", "0.3"]);
        assert_eq!(sample.mean().value().unwrap().as_str(), "0.2");
        let sample = stats(&["1", "2", "4", "100"]);
        assert_eq!(sample.median().value().unwrap().as_str(), "3");
        let maximum = Decimal::MAX.to_string();
        let sample = stats(&[&maximum, &maximum, &maximum, &maximum]);
        assert_eq!(sample.mean().value().unwrap().decimal(), Decimal::MAX);
        assert_eq!(sample.median().value().unwrap().decimal(), Decimal::MAX);
    }
}
