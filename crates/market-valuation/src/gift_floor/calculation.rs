//! Additive premiums over one coherent collection/currency floor generation.
use super::{GiftFloorBasis, GiftFloorValuation};
use crate::floors::{Floor, Statistics};
use crate::types::*;
use market_gifts::*;
use rust_decimal::Decimal;

impl Statistics {
    pub fn report(&self, traits: &GiftTraits, pair: &Floor) -> GiftFloorValuation {
        let empty = Floor::default();
        let model = traits
            .model
            .as_ref()
            .filter(|id| self.known_models.contains(*id));
        let backdrop = traits
            .backdrop
            .as_ref()
            .filter(|id| self.known_backdrops.contains(*id));
        let model_floor = model.and_then(|id| self.models.get(id)).unwrap_or(&empty);
        let backdrop_floor = backdrop
            .and_then(|id| self.backdrops.get(id))
            .unwrap_or(&empty);
        calculate(
            self.collection.valuation(true),
            model_floor.valuation(model.is_some()),
            backdrop_floor.valuation(backdrop.is_some()),
            pair.valuation(model.is_some() && backdrop.is_some()),
        )
    }
}

type Calculation = Result<Decimal, ValuationIssue>;
fn value(metric: &ValuationMetric, dependency: ValuationDependency) -> Calculation {
    match metric {
        ValuationMetric::Value { value } => Ok(value.decimal()),
        ValuationMetric::Unavailable { error } => {
            let mut error = error.clone();
            error.dependency = Some(dependency);
            Err(error)
        }
    }
}
fn overflow() -> ValuationIssue {
    ValuationIssue {
        code: ValuationIssueCode::CalculationOverflow,
        message: "Calculated value is outside the supported decimal range".into(),
        required_samples: None,
        available_samples: None,
        dependency: None,
    }
}
fn metric<T: TryFrom<Decimal>>(value: Calculation, rounded: bool) -> ValuationMetric<T> {
    match value.and_then(|value| {
        T::try_from(if rounded { value.round_dp(2) } else { value }).map_err(|_| overflow())
    }) {
        Ok(value) => ValuationMetric::Value { value },
        Err(error) => ValuationMetric::Unavailable { error },
    }
}
fn premium(base: &Calculation, selected: &Calculation) -> Calculation {
    selected
        .clone()?
        .checked_sub(base.clone()?)
        .map(|p| p.max(Decimal::ZERO))
        .ok_or_else(overflow)
}
fn ratio(base: &Calculation, selected: &Calculation) -> Calculation {
    selected
        .clone()?
        .checked_div(base.clone()?)
        .ok_or_else(overflow)
}
fn calculate(
    collection_floor: crate::FloorValuation,
    model_floor: crate::FloorValuation,
    backdrop_floor: crate::FloorValuation,
    model_backdrop_floor: crate::FloorValuation,
) -> GiftFloorValuation {
    let base = value(
        &collection_floor.price,
        ValuationDependency::CollectionFloor,
    )
    .and_then(|base| {
        if base > Decimal::ZERO {
            Ok(base)
        } else {
            Err(ValuationIssue {
                code: ValuationIssueCode::InvalidBaseline,
                message: "Collection floor must be positive".into(),
                required_samples: None,
                available_samples: None,
                dependency: Some(ValuationDependency::CollectionFloor),
            })
        }
    });
    let model = value(&model_floor.price, ValuationDependency::ModelFloor);
    let backdrop = value(&backdrop_floor.price, ValuationDependency::BackdropFloor);
    let model_premium = premium(&base, &model);
    let backdrop_premium = premium(&base, &backdrop);
    let estimate = (|| {
        let base = base.clone()?;
        let model_premium = model_premium.clone()?;
        let backdrop_premium = backdrop_premium.clone()?;
        base.checked_add(model_premium)
            .and_then(|value| value.checked_add(backdrop_premium))
            .ok_or_else(overflow)
    })();
    let cap = model_backdrop_floor.value().map(Amount::decimal);
    let mark = match estimate.clone() {
        Ok(estimate) => Ok(cap.map_or(estimate, |cap| estimate.min(cap))),
        // All summands are nonnegative: an overflowing sum exceeds a representable cap.
        Err(error) if error.code == ValuationIssueCode::CalculationOverflow && cap.is_some() => {
            Ok(cap.unwrap())
        }
        Err(error) => Err(error),
    };
    let final_mark = metric(mark, true);
    let basis = final_mark.value().map(|_| {
        if cap.is_some() {
            GiftFloorBasis::PairCapped
        } else {
            GiftFloorBasis::Additive
        }
    });
    GiftFloorValuation {
        model_multiplier: metric(ratio(&base, &model), true),
        backdrop_multiplier: metric(ratio(&base, &backdrop), true),
        model_premium: metric(model_premium, false),
        backdrop_premium: metric(backdrop_premium, false),
        additive_estimate: metric(estimate, false),
        final_mark,
        basis,
        collection_floor,
        model_floor,
        backdrop_floor,
        model_backdrop_floor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn floor(price: &str) -> crate::FloorValuation {
        crate::FloorValuation {
            price: ValuationMetric::Value {
                value: Amount::new(price).unwrap(),
            },
            count: 0,
        }
    }
    fn missing() -> crate::FloorValuation {
        crate::FloorValuation {
            count: 0,
            price: crate::market::unavailable(
                ValuationIssueCode::NotEnoughData,
                "No eligible asks",
                None,
                None,
            ),
        }
    }
    fn report(c: &str, m: &str, b: &str, pair: Option<&str>) -> GiftFloorValuation {
        calculate(
            floor(c),
            floor(m),
            floor(b),
            pair.map(floor).unwrap_or_else(missing),
        )
    }
    #[test]
    fn additive_premiums_limit_baseline_sensitivity_without_sample_gates() {
        for (base, expected) in [("3", "18"), ("1", "20")] {
            let result = report(base, "10", "11", Some("100"));
            assert_eq!(result.final_mark.value().unwrap().as_str(), expected);
            assert_eq!(result.basis, Some(GiftFloorBasis::PairCapped));
        }
        let result = report("3", "10", "11", None);
        assert_eq!(result.model_premium.value().unwrap().as_str(), "7");
        assert_eq!(result.backdrop_premium.value().unwrap().as_str(), "8");
        assert_eq!(result.model_multiplier.value().unwrap().as_str(), "3.33");
        assert_eq!(result.backdrop_multiplier.value().unwrap().as_str(), "3.67");
        assert_eq!(result.final_mark.value().unwrap().as_str(), "18");
        assert_eq!(result.basis, Some(GiftFloorBasis::Additive));
        assert!(result.model_backdrop_floor.value().is_none());
    }
    #[test]
    fn premiums_are_nonnegative_and_rounding_follows_the_optional_pair_cap() {
        assert_eq!(
            report("10", "8", "9", None)
                .final_mark
                .value()
                .unwrap()
                .as_str(),
            "10"
        );
        for (pair, expected) in [("15.125", "15.12"), ("15.135", "15.14")] {
            let result = report("3", "10", "11", Some(pair));
            assert_eq!(result.final_mark.value().unwrap().as_str(), expected);
            assert_eq!(result.model_backdrop_floor.value().unwrap().as_str(), pair);
        }
        let result = report("3", "10", "3.7065", None);
        assert_eq!(
            result.additive_estimate.value().unwrap().as_str(),
            "10.7065"
        );
        assert_eq!(result.final_mark.value().unwrap().as_str(), "10.71");
    }
    #[test]
    fn missing_inputs_preserve_errors_but_missing_pairs_do_not_block_estimates() {
        let mut result = report("0", "10", "11", Some("100"));
        let ValuationMetric::Unavailable { error } = &result.final_mark else {
            panic!("baseline")
        };
        assert_eq!(error.code, ValuationIssueCode::InvalidBaseline);
        assert_eq!(error.dependency, Some(ValuationDependency::CollectionFloor));
        result = calculate(floor("3"), missing(), floor("11"), floor("100"));
        let ValuationMetric::Unavailable { error } = &result.final_mark else {
            panic!("model floor")
        };
        assert_eq!(error.code, ValuationIssueCode::NotEnoughData);
        assert_eq!(error.dependency, Some(ValuationDependency::ModelFloor));
        assert_eq!(result.basis, None);
    }
    #[test]
    fn explanatory_ratio_overflow_cannot_block_a_valid_mark() {
        let result = report("0.0000000000000000000000000001", "100", "100", None);
        assert!(result.model_multiplier.value().is_none());
        assert_eq!(result.final_mark.value().unwrap().as_str(), "200");
    }
    #[test]
    fn positive_sum_overflow_uses_a_valid_pair_cap_or_reports_failure() {
        let max = Decimal::MAX.to_string();
        let result = report("1", &max, &max, Some("20"));
        assert!(result.additive_estimate.value().is_none());
        assert_eq!(result.final_mark.value().unwrap().as_str(), "20");
        assert!(report("1", &max, &max, None).final_mark.value().is_none());
    }
}
