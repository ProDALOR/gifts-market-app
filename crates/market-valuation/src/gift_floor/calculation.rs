//! Pure floor, multiplier and composite calculations over one coherent statistics set.
use super::{GiftFloorEvidence, GiftFloorValuation, MINIMUM_MULTIPLIER_GIFTS};
use crate::floors::{Floor, Statistics};
use crate::types::*;
use market_gifts::*;
use rust_decimal::Decimal;
impl Statistics {
    pub fn report(
        &self,
        gift: CollectibleGift,
        lookup: GiftLookup,
        currency: CurrencyId,
    ) -> GiftFloorValuation {
        let empty = Floor::default();
        let model = gift
            .traits
            .model
            .as_ref()
            .filter(|id| self.known_models.contains(*id));
        let backdrop = gift
            .traits
            .backdrop
            .as_ref()
            .filter(|id| self.known_backdrops.contains(*id));
        let model_floor = model.and_then(|id| self.models.get(id)).unwrap_or(&empty);
        let backdrop_floor = backdrop
            .and_then(|id| self.backdrops.get(id))
            .unwrap_or(&empty);
        let pair = model
            .zip(backdrop)
            .and_then(|(m, b)| self.pairs.get(&(m.clone(), b.clone())))
            .unwrap_or(&empty);
        let model_ratio = ratio(&self.collection, model_floor, model.is_some());
        let backdrop_ratio = ratio(&self.collection, backdrop_floor, backdrop.is_some());
        let final_mark = final_mark(
            pair.price.as_ref(),
            &self.collection,
            model_floor,
            backdrop_floor,
            &model_ratio,
            &backdrop_ratio,
        );
        GiftFloorValuation {
            slug: match lookup {
                GiftLookup::Slug(slug) => Some(slug),
                GiftLookup::Catalog { .. } => None,
            },
            currency_id: currency,
            collection_floor: self.collection.metric(true),
            model_floor: model_floor.metric(model.is_some()),
            backdrop_floor: backdrop_floor.metric(backdrop.is_some()),
            model_backdrop_floor: pair.metric(model.is_some() && backdrop.is_some()),
            model_multiplier: rounded_metric(model_ratio),
            backdrop_multiplier: rounded_metric(backdrop_ratio),
            final_mark,
            evidence: GiftFloorEvidence {
                collection: self.collection.evidence(),
                model: model_floor.evidence(),
                backdrop: backdrop_floor.evidence(),
                model_backdrop: pair.evidence(),
                minimum_multiplier_gifts: MINIMUM_MULTIPLIER_GIFTS,
            },
            gift,
        }
    }
}
type Ratio = Result<Decimal, ValuationIssue>;
fn ratio(collection: &Floor, selected: &Floor, present: bool) -> Ratio {
    let error = |code, message: &str| ValuationIssue {
        code,
        message: message.into(),
        required_samples: (code == ValuationIssueCode::NotEnoughData)
            .then_some(MINIMUM_MULTIPLIER_GIFTS),
        available_samples: (code == ValuationIssueCode::NotEnoughData)
            .then_some(collection.evidence().gifts.min(selected.evidence().gifts)),
        dependency: None,
    };
    if !present {
        return Err(error(
            ValuationIssueCode::MissingTrait,
            "Gift has no resolved catalog trait",
        ));
    }
    if collection.evidence().gifts < MINIMUM_MULTIPLIER_GIFTS
        || selected.evidence().gifts < MINIMUM_MULTIPLIER_GIFTS
    {
        return Err(error(
            ValuationIssueCode::NotEnoughData,
            "At least three distinct identifiable gifts are required for the collection and selected trait",
        ));
    }
    let Some(baseline) = collection
        .price
        .as_ref()
        .map(Amount::decimal)
        .filter(|p| *p > Decimal::ZERO)
    else {
        return Err(error(
            ValuationIssueCode::InvalidBaseline,
            "Collection floor must be positive",
        ));
    };
    selected
        .price
        .as_ref()
        .and_then(|p| p.decimal().checked_div(baseline))
        .ok_or_else(|| {
            error(
                ValuationIssueCode::CalculationOverflow,
                "Multiplier calculation overflow",
            )
        })
}
fn final_mark(
    combined: Option<&Amount>,
    collection: &Floor,
    model_floor: &Floor,
    backdrop_floor: &Floor,
    model: &Ratio,
    backdrop: &Ratio,
) -> ValuationMetric {
    for (error, dependency, label) in [
        (
            model.as_ref().err(),
            ValuationDependency::ModelMultiplier,
            "Model multiplier",
        ),
        (
            backdrop.as_ref().err(),
            ValuationDependency::BackdropMultiplier,
            "Backdrop multiplier",
        ),
    ] {
        if let Some(error) = error {
            let mut error = error.clone();
            error.dependency = Some(dependency);
            error.message = format!("{label}: {}", error.message);
            return ValuationMetric::Unavailable { error };
        }
    }
    let (Some(combined), Some(base), Some(model_price), Some(backdrop_price), Ok(_), Ok(backdrop)) = (
        combined,
        &collection.price,
        &model_floor.price,
        &backdrop_floor.price,
        model,
        backdrop,
    ) else {
        return ValuationMetric::Unavailable {
            error: ValuationIssue {
                code: ValuationIssueCode::NotEnoughData,
                message: "Model × backdrop floor: no eligible matching asks".into(),
                required_samples: Some(1),
                available_samples: Some(0),
                dependency: Some(ValuationDependency::ModelBackdropFloor),
            },
        };
    };
    // Algebraically identical to base * (model/base) * (backdrop/base),
    // without reconstructing prices from non-terminating intermediate ratios.
    let estimate = model_price
        .decimal()
        .checked_mul(backdrop_price.decimal())
        .and_then(|v| v.checked_div(base.decimal()))
        .or_else(|| model_price.decimal().checked_mul(*backdrop));
    // All operands are positive. If even the scaled multiplication overflows,
    // its value exceeds any representable combined floor, which therefore wins.
    rounded_metric(Ok(estimate.map_or_else(
        || combined.decimal(),
        |estimate| combined.decimal().min(estimate),
    )))
}
fn rounded_metric<T: TryFrom<Decimal>>(value: Ratio) -> ValuationMetric<T> {
    match value {
        Ok(value) => match T::try_from(value.round_dp(2)) {
            Ok(value) => ValuationMetric::Value { value },
            Err(_) => issue(
                ValuationIssueCode::CalculationOverflow,
                "Rounded value is outside the supported range",
            ),
        },
        Err(error) => ValuationMetric::Unavailable { error },
    }
}
fn issue<T>(code: ValuationIssueCode, message: &str) -> ValuationMetric<T> {
    ValuationMetric::Unavailable {
        error: ValuationIssue {
            code,
            message: message.into(),
            required_samples: None,
            available_samples: None,
            dependency: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn floor(price: &str, samples: u32) -> Floor {
        Floor::fixture(price, samples)
    }
    #[test]
    fn rounding_is_only_at_the_boundary_and_minimum_precedes_rounding() {
        let collection = floor("3", 3);
        let model = ratio(&collection, &floor("10", 3), true);
        let backdrop = ratio(&collection, &floor("11", 3), true);
        assert_eq!(
            rounded_metric::<Multiplier>(model.clone())
                .value()
                .unwrap()
                .as_str(),
            "3.33"
        );
        assert_eq!(
            rounded_metric::<Multiplier>(backdrop.clone())
                .value()
                .unwrap()
                .as_str(),
            "3.67"
        );
        assert_eq!(
            final_mark(
                Some(&"100".into()),
                &collection,
                &floor("10", 3),
                &floor("11", 3),
                &model,
                &backdrop
            )
            .value()
            .unwrap()
            .as_str(),
            "36.67"
        );
        assert_eq!(
            final_mark(
                Some(&"20.125".into()),
                &collection,
                &floor("10", 3),
                &floor("11", 3),
                &model,
                &backdrop
            )
            .value()
            .unwrap()
            .as_str(),
            "20.12"
        );
    }
    #[test]
    fn repeating_intermediate_ratios_do_not_change_a_rounding_tie() {
        let base = floor("3", 3);
        let model = floor("10", 3);
        let backdrop = floor("3.7065", 3);
        let mark = final_mark(
            Some(&"100".into()),
            &base,
            &model,
            &backdrop,
            &ratio(&base, &model, true),
            &ratio(&base, &backdrop, true),
        );
        assert_eq!(mark.value().unwrap().as_str(), "12.36");
    }
    #[test]
    fn missing_evidence_and_zero_baselines_never_become_zero_marks() {
        assert!(ratio(&floor("3", 3), &floor("10", 2), true).is_err());
        assert!(matches!(
            ratio(&floor("0", 3), &floor("10", 3), true),
            Err(ValuationIssue {
                code: ValuationIssueCode::InvalidBaseline,
                ..
            })
        ));
        let ratio = Ok(Decimal::ONE);
        assert!(
            final_mark(
                None,
                &floor("1", 3),
                &floor("1", 3),
                &floor("1", 3),
                &ratio,
                &ratio
            )
            .value()
            .is_none()
        );
    }

    #[test]
    fn final_errors_preserve_dependency_code_and_distinct_gift_evidence() {
        let base = floor("3", 3);
        let model = floor("10", 3);
        let backdrop = floor("11", 2);
        for (present, code) in [
            (false, ValuationIssueCode::MissingTrait),
            (true, ValuationIssueCode::NotEnoughData),
        ] {
            let metric = final_mark(
                Some(&"100".into()),
                &base,
                &model,
                &backdrop,
                &ratio(&base, &model, true),
                &ratio(&base, &backdrop, present),
            );
            let ValuationMetric::Unavailable { error } = metric else {
                panic!("missing evidence must be explicit")
            };
            assert_eq!(error.code, code);
            assert_eq!(
                error.dependency,
                Some(ValuationDependency::BackdropMultiplier)
            );
            assert_eq!(error.required_samples, present.then_some(3));
            assert_eq!(error.available_samples, present.then_some(2));
        }
        let metric = final_mark(
            None,
            &base,
            &model,
            &model,
            &Ok(Decimal::ONE),
            &Ok(Decimal::ONE),
        );
        let ValuationMetric::Unavailable { error } = metric else {
            panic!("missing pair floor must be explicit")
        };
        assert_eq!(
            error.dependency,
            Some(ValuationDependency::ModelBackdropFloor)
        );
    }

    #[test]
    fn baseline_sensitivity_is_explicit_and_the_matching_floor_caps_the_heuristic() {
        let model = floor("10", 3);
        let backdrop = floor("11", 3);
        for (price, expected) in [("3", "36.67"), ("1", "100")] {
            let base = floor(price, 3);
            let mark = final_mark(
                Some(&"100".into()),
                &base,
                &model,
                &backdrop,
                &ratio(&base, &model, true),
                &ratio(&base, &backdrop, true),
            );
            assert_eq!(mark.value().unwrap().as_str(), expected);
        }
    }
}
