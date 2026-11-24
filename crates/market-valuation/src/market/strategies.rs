//! Strategies consume checked input values and retain their own typed outputs.
use super::{
    execution::{MAX_OBSERVATIONS, eligible},
    *,
};
use crate::Strategy;
use market_gifts::{GiftTarget, OrderSource};
use std::sync::Arc;
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
    const NAME: &'static str = "Best ask";
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
        context.matching_floor(target)
    }
}
pub(crate) fn matching_ask_floor(
    context: &ValuationContext<'_>,
    target: &ResolvedTarget,
) -> Result<FloorValuation, ValuationError> {
    let query = target.query(context.currency().clone(), OrderSource::Asks, None);
    let mut token = String::new();
    let mut floor = crate::floors::Floor::default();
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
            if floor.observations as usize >= MAX_OBSERVATIONS {
                return Err(ValuationError::capacity("ask observation budget exceeded"));
            }
            let gift = match &order.target {
                GiftTarget::Gift { gift } => Some(Arc::new(gift.id.clone())),
                _ => None,
            };
            floor.observe(&order.price, gift.as_ref());
        }
        if page.next_page_token.is_empty() {
            break;
        }
        token = page.next_page_token;
    }
    Ok(floor.valuation(true))
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
