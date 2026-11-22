//! Strategies consume checked input values and retain their own typed outputs.
use super::{
    execution::{MAX_OBSERVATIONS, eligible},
    *,
};
use crate::Strategy;
use market_gifts::{GiftTarget, OrderSource};
use std::collections::BTreeSet;
pub struct AskFloor;
impl Strategy for AskFloor {
    const PURPOSE: crate::StrategyPurpose = crate::StrategyPurpose::Reference;
    const DISPLAY: crate::DisplayDefinition = crate::DisplayDefinition {
        fields: &[crate::FieldDefinition::currency("price", "Value", None)
            .at("")
            .primary()],
        evidence: &[crate::EvidenceDefinition {
            path: "/count",
            label: "gifts",
        }],
        note: "Lowest eligible ask, before fees.",
    };
    type Input = SelectionInput;
    const ID: &'static str = "ask_floor";
    const NAME: &'static str = "Ask floor";
    const DESCRIPTION: &'static str =
        "Lowest eligible ask for a collection, trait selection or individual gift.";
    const DOCUMENTATION: &'static str = include_str!("ask_floor.md");
    type Output = FloorValuation;
    fn evaluate(
        &self,
        input: &Self::Input,
        context: &ValuationContext<'_>,
    ) -> Result<Self::Output, ValuationError> {
        let target = input.target();
        if target.simple_floor() {
            let collection = target.collection().expect("single selection");
            let selection = match (target.model(), target.backdrop()) {
                (Some(model), None) => crate::floors::FloorSelection::Model(model),
                (None, Some(backdrop)) => crate::floors::FloorSelection::Backdrop(backdrop),
                _ => crate::floors::FloorSelection::Collection,
            };
            let stats = if matches!(selection, crate::floors::FloorSelection::Collection) {
                context.collection_statistics(collection)?
            } else {
                context.floor_statistics(collection)?
            };
            return Ok(stats.selection(selection));
        }
        matching_ask_floor(context, target)
    }
}
pub(crate) fn matching_ask_floor(
    context: &ValuationContext<'_>,
    target: &ResolvedTarget,
) -> Result<FloorValuation, ValuationError> {
    let query = target.query(context.currency().clone(), OrderSource::Asks, None);
    let mut token = String::new();
    let mut floor: Option<Amount> = None;
    let mut gifts = BTreeSet::new();
    let mut count = 0;
    let mut budget = market_replica::catalog_read::ScanBudget::default();
    loop {
        budget.page()?;
        let page = context.orders(&query, token)?;
        for order in page.data {
            context.snapshot().check_cancelled()?;
            if order
                .expires_at
                .is_some_and(|expiry| expiry <= jiff::Timestamp::now())
                || !eligible(&order, target)
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
    Ok(FloorValuation {
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
        count: gifts.len() as u32,
    })
}
pub struct ExecutionMean;
impl Strategy for ExecutionMean {
    const DISPLAY: crate::DisplayDefinition = crate::DisplayDefinition {
        fields: &[crate::FieldDefinition::currency("price", "Value", Some(2)).primary()],
        evidence: &[
            crate::EvidenceDefinition {
                path: "/evidence/executions",
                label: "executions",
            },
            crate::EvidenceDefinition {
                path: "/evidence/gifts",
                label: "gifts",
            },
            crate::EvidenceDefinition {
                path: "/evidence/venues",
                label: "venues",
            },
            crate::EvidenceDefinition {
                path: "/evidence/first_execution",
                label: "first execution",
            },
            crate::EvidenceDefinition {
                path: "/evidence/last_execution",
                label: "last execution",
            },
        ],
        note: "Unweighted arithmetic mean per execution record; observed history only.",
    };
    type Input = ExecutionInput;
    const ID: &'static str = "execution_mean";
    const NAME: &'static str = "Execution mean";
    const DESCRIPTION: &'static str =
        "Unweighted arithmetic mean of observed per-gift execution prices in an explicit period.";
    const DOCUMENTATION: &'static str = include_str!("executions.md");
    type Output = ExecutionPriceValuation;
    fn evaluate(
        &self,
        input: &Self::Input,
        context: &ValuationContext<'_>,
    ) -> Result<Self::Output, ValuationError> {
        let stats = context.executions(input)?;
        Ok(ExecutionPriceValuation {
            price: stats.mean(),
            period: input.period().as_period(),
            evidence: stats.evidence.clone(),
        })
    }
}
pub struct ExecutionMedian;
impl Strategy for ExecutionMedian {
    const DISPLAY: crate::DisplayDefinition = crate::DisplayDefinition {
        fields: &[crate::FieldDefinition::currency("price", "Value", Some(2)).primary()],
        evidence: &[
            crate::EvidenceDefinition {
                path: "/evidence/executions",
                label: "executions",
            },
            crate::EvidenceDefinition {
                path: "/evidence/gifts",
                label: "gifts",
            },
            crate::EvidenceDefinition {
                path: "/evidence/venues",
                label: "venues",
            },
            crate::EvidenceDefinition {
                path: "/evidence/first_execution",
                label: "first execution",
            },
            crate::EvidenceDefinition {
                path: "/evidence/last_execution",
                label: "last execution",
            },
        ],
        note: "Median per execution record; observed history only.",
    };
    type Input = ExecutionInput;
    const ID: &'static str = "execution_median";
    const NAME: &'static str = "Execution median";
    const DESCRIPTION: &'static str =
        "Median observed per-gift execution price in an explicit period.";
    const DOCUMENTATION: &'static str = include_str!("executions.md");
    type Output = ExecutionPriceValuation;
    fn evaluate(
        &self,
        input: &Self::Input,
        context: &ValuationContext<'_>,
    ) -> Result<Self::Output, ValuationError> {
        let stats = context.executions(input)?;
        Ok(ExecutionPriceValuation {
            price: stats.median(),
            period: input.period().as_period(),
            evidence: stats.evidence.clone(),
        })
    }
}
