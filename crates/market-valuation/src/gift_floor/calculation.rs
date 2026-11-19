//! One pass builds all price evidence. Rounding happens only at the report boundary.
use super::{GiftFloorEvidence, GiftFloorValuation, MINIMUM_MULTIPLIER_SAMPLES};
use crate::ValuationError;
use crate::source::{BookScope, CatalogTrait, ReadDependencies, ValuationSnapshot};
use crate::types::*;
use market_gifts::*;
use rust_decimal::Decimal;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::time::Instant;

pub(crate) const REFRESH: Duration = Duration::from_secs(30);
const STAT_LIMIT: usize = 20_000;
#[derive(Default, Clone)]
pub(crate) struct Floor {
    price: Option<Amount>,
    samples: u32,
}
impl Floor {
    fn observe(&mut self, price: &Amount) {
        self.samples += 1;
        if self
            .price
            .as_ref()
            .is_none_or(|old| price.decimal() < old.decimal())
        {
            self.price = Some(price.clone());
        }
    }
    fn metric(&self, present: bool) -> ValuationMetric {
        if !present {
            return issue(
                ValuationIssueCode::MissingTrait,
                "Gift has no resolved catalog trait",
            );
        }
        match &self.price {
            Some(value) => ValuationMetric::Value {
                value: value.clone(),
            },
            None => issue(
                ValuationIssueCode::NotEnoughData,
                "No eligible asks match this selection",
            ),
        }
    }
}
pub(crate) struct Statistics {
    known_models: BTreeSet<ModelId>,
    known_backdrops: BTreeSet<BackdropId>,
    collection: Floor,
    models: BTreeMap<ModelId, Floor>,
    backdrops: BTreeMap<BackdropId, Floor>,
    pairs: BTreeMap<(ModelId, BackdropId), Floor>,
    dependencies: ReadDependencies,
    created: Instant,
    expires_at_ms: i64,
}
impl Statistics {
    pub fn current(&self, snapshot: &ValuationSnapshot) -> Result<bool, ValuationError> {
        Ok(self.created.elapsed() < REFRESH
            && snapshot.dependencies_current(&self.dependencies)?
            && self.created.elapsed() < REFRESH
            && jiff::Timestamp::now().as_millisecond() < self.expires_at_ms)
    }
    pub fn read(snapshot: &ValuationSnapshot, scope: &BookScope) -> Result<Self, ValuationError> {
        snapshot.require_currency(&scope.currency)?;
        let mut stats = Self {
            known_models: snapshot.model_ids(&scope.collection)?,
            known_backdrops: snapshot.backdrop_ids(&scope.collection)?,
            collection: Floor::default(),
            models: BTreeMap::new(),
            backdrops: BTreeMap::new(),
            pairs: BTreeMap::new(),
            dependencies: snapshot.dependencies(
                [scope.clone()],
                &scope.currency,
                &[CatalogTrait::Model, CatalogTrait::Backdrop],
                &[],
            )?,
            created: Instant::now(),
            expires_at_ms: i64::MAX,
        };
        let now = jiff::Timestamp::now().as_millisecond();
        if stats.known_models.len() + stats.known_backdrops.len() > STAT_LIMIT {
            return Err(ValuationError::capacity(
                "Gift valuation statistics budget exceeded",
            ));
        }
        let mut token = String::new();
        let mut budget = market_replica::catalog_read::ScanBudget::default();
        loop {
            budget.page()?;
            let page = snapshot.asks(scope, now, token)?;
            for quote in page.data {
                stats.collection.observe(&quote.price);
                if let Some(at) = quote.expires_at {
                    stats.expires_at_ms = stats.expires_at_ms.min(at.as_millisecond());
                }
                let model = quote.model_id.filter(|id| stats.known_models.contains(id));
                let backdrop = quote
                    .backdrop_id
                    .filter(|id| stats.known_backdrops.contains(id));
                if let Some(id) = &model {
                    stats
                        .models
                        .entry(id.clone())
                        .or_default()
                        .observe(&quote.price);
                }
                if let Some(id) = &backdrop {
                    stats
                        .backdrops
                        .entry(id.clone())
                        .or_default()
                        .observe(&quote.price);
                }
                if let Some(pair) = model.zip(backdrop) {
                    stats.pairs.entry(pair).or_default().observe(&quote.price);
                }
                if stats.models.len()
                    + stats.backdrops.len()
                    + stats.pairs.len()
                    + stats.known_models.len()
                    + stats.known_backdrops.len()
                    > STAT_LIMIT
                {
                    return Err(ValuationError::capacity(
                        "Gift valuation statistics budget exceeded",
                    ));
                }
            }
            if page.next_page_token.is_empty() {
                return Ok(stats);
            }
            token = page.next_page_token;
        }
    }
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
                collection_samples: self.collection.samples,
                model_samples: model_floor.samples,
                backdrop_samples: backdrop_floor.samples,
                model_backdrop_samples: pair.samples,
                minimum_multiplier_samples: MINIMUM_MULTIPLIER_SAMPLES,
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
        required_samples: Some(MINIMUM_MULTIPLIER_SAMPLES),
        available_samples: Some(collection.samples.min(selected.samples)),
    };
    if !present {
        return Err(error(
            ValuationIssueCode::MissingTrait,
            "Gift has no resolved catalog trait",
        ));
    }
    if collection.samples < MINIMUM_MULTIPLIER_SAMPLES
        || selected.samples < MINIMUM_MULTIPLIER_SAMPLES
    {
        return Err(error(
            ValuationIssueCode::NotEnoughData,
            "At least three eligible asks are required for the collection and selected trait",
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
    for error in [model.as_ref().err(), backdrop.as_ref().err()]
        .into_iter()
        .flatten()
    {
        if matches!(
            error.code,
            ValuationIssueCode::InvalidBaseline | ValuationIssueCode::CalculationOverflow
        ) {
            return ValuationMetric::Unavailable {
                error: error.clone(),
            };
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
        return issue(
            ValuationIssueCode::NotEnoughData,
            "Final mark requires the combined floor and both multipliers",
        );
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
fn rounded_metric(value: Ratio) -> ValuationMetric {
    match value {
        Ok(value) => match Amount::try_from(value.round_dp(2)) {
            Ok(value) => ValuationMetric::Value { value },
            Err(_) => issue(
                ValuationIssueCode::CalculationOverflow,
                "Rounded value is outside the supported range",
            ),
        },
        Err(error) => ValuationMetric::Unavailable { error },
    }
}
fn issue(code: ValuationIssueCode, message: &str) -> ValuationMetric {
    ValuationMetric::Unavailable {
        error: ValuationIssue {
            code,
            message: message.into(),
            required_samples: None,
            available_samples: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn floor(price: &str, samples: u32) -> Floor {
        Floor {
            price: Some(Amount::new(price).unwrap()),
            samples,
        }
    }
    #[test]
    fn rounding_is_only_at_the_boundary_and_minimum_precedes_rounding() {
        let collection = floor("3", 3);
        let model = ratio(&collection, &floor("10", 3), true);
        let backdrop = ratio(&collection, &floor("11", 3), true);
        assert_eq!(
            rounded_metric(model.clone()).value().unwrap().as_str(),
            "3.33"
        );
        assert_eq!(
            rounded_metric(backdrop.clone()).value().unwrap().as_str(),
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
}
