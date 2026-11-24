//! Observed bid quotes and conservative trait references, evaluated against one pinned book.
use crate::{market::unavailable, *};
use market_gifts::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct BidOrder {
    pub id: OrderId,
    pub venue_id: VenueId,
    pub quantity: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature="schema", schema(value_type=Option<String>,format=DateTime))]
    pub expires_at: Option<jiff::Timestamp>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct BidQuote {
    #[serde(flatten)]
    pub price: ValuationMetric,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<BidOrder>,
}
impl BidQuote {
    fn from_order(order: Option<&GiftOrder>) -> Self {
        match order {
            Some(order) => Self {
                price: ValuationMetric::Value {
                    value: order.price.clone(),
                },
                order: Some(BidOrder {
                    id: order.id.clone(),
                    venue_id: order.book.venue_id.clone(),
                    quantity: order.quantity,
                    expires_at: order.expires_at,
                }),
            },
            None => Self {
                price: unavailable(
                    ValuationIssueCode::NotEnoughData,
                    "No eligible matching bid",
                    Some(1),
                    Some(0),
                ),
                order: None,
            },
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct BidReferences {
    pub collection: BidQuote,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<BidQuote>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backdrop: Option<BidQuote>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_backdrop: Option<BidQuote>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<BidQuote>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct BestBidValuation {
    pub best: BidQuote,
    /// Unspecified dimensions must stay unrestricted in these independent references.
    pub references: BidReferences,
}
pub struct BestBid;
impl Strategy for BestBid {
    type Input = SelectionInput;
    type Output = BestBidValuation;
    const ID: &'static str = "best_bid";
    const NAME: &'static str = "Best bid";
    const DESCRIPTION: &'static str =
        "Highest eligible bid accepting the gift or covering the complete selection.";
    const DOCUMENTATION: &'static str = include_str!("best_bid.md");
    const PURPOSE: StrategyPurpose = StrategyPurpose::Quote;
    const DISPLAY: DisplayDefinition = DisplayDefinition {
        fields: &[FieldDefinition::currency("best", "Best bid", Some(2)).primary()],
        evidence: &[],
        note: "Observed bid before fees; venue conditions and custody requirements still apply.",
    };
    fn evaluate(
        &self,
        input: &SelectionInput,
        context: &ValuationContext<'_>,
    ) -> Result<Self::Output, ValuationError> {
        let target = input.target();
        let orders = context.bid_orders(target.collection().expect("single selection"))?;
        let now = jiff::Timestamp::now();
        let find = |accept: &dyn Fn(&GiftTarget) -> bool| -> Result<BidQuote, ValuationError> {
            for order in orders.iter() {
                context.snapshot().check_cancelled()?;
                if order.expires_at.is_none_or(|at| at > now) && accept(&order.target) {
                    return Ok(BidQuote::from_order(Some(order)));
                }
            }
            Ok(BidQuote::from_order(None))
        };
        let best = find(&|bid| matches_target(bid, target))?;
        let selected = GiftTraits {
            model: target.model().cloned(),
            backdrop: target.backdrop().cloned(),
            pattern: target.pattern().cloned(),
        };
        let traits = match target {
            ResolvedTarget::Gift { gift, .. } => &gift.traits,
            ResolvedTarget::GiftAttributes(gift) => &gift.traits,
            _ => &selected,
        };
        let references = {
            let reference = |criteria: GiftCriteria| {
                find(
                    &|bid| matches!(bid, GiftTarget::Criteria { criteria: accepted } if accepted.covers(&criteria)),
                )
            };
            BidReferences {
                collection: reference(GiftCriteria::default())?,
                model: traits
                    .model
                    .as_ref()
                    .map(|id| {
                        reference(GiftCriteria {
                            models: vec![id.clone()],
                            ..Default::default()
                        })
                    })
                    .transpose()?,
                backdrop: traits
                    .backdrop
                    .as_ref()
                    .map(|id| {
                        reference(GiftCriteria {
                            backdrops: vec![id.clone()],
                            ..Default::default()
                        })
                    })
                    .transpose()?,
                model_backdrop: traits
                    .model
                    .as_ref()
                    .zip(traits.backdrop.as_ref())
                    .map(|(m, b)| {
                        reference(GiftCriteria {
                            models: vec![m.clone()],
                            backdrops: vec![b.clone()],
                            ..Default::default()
                        })
                    })
                    .transpose()?,
                pattern: traits
                    .pattern
                    .as_ref()
                    .map(|id| {
                        reference(GiftCriteria {
                            patterns: vec![id.clone()],
                            ..Default::default()
                        })
                    })
                    .transpose()?,
            }
        };
        Ok(BestBidValuation { best, references })
    }
}
fn matches_target(bid: &GiftTarget, target: &ResolvedTarget) -> bool {
    match bid {
        GiftTarget::Unsupported { .. } => false,
        GiftTarget::Criteria { criteria } => match target {
            ResolvedTarget::Selection {
                criteria: selected, ..
            } => criteria.covers(selected),
            ResolvedTarget::Gift { gift, .. } => criteria.accepts(&gift.traits, gift.number),
            ResolvedTarget::GiftAttributes(gift) => criteria.accepts(&gift.traits, gift.number),
            _ => false,
        },
        GiftTarget::Gift { gift: bid } => match target {
            ResolvedTarget::Gift { gift, .. } => gift.id == bid.id,
            ResolvedTarget::GiftAttributes(gift) => {
                gift.number.is_some()
                    && gift.number == bid.number
                    && gift
                        .traits
                        .model
                        .as_ref()
                        .is_none_or(|id| bid.traits.model.as_ref() == Some(id))
                    && gift
                        .traits
                        .backdrop
                        .as_ref()
                        .is_none_or(|id| bid.traits.backdrop.as_ref() == Some(id))
                    && gift
                        .traits
                        .pattern
                        .as_ref()
                        .is_none_or(|id| bid.traits.pattern.as_ref() == Some(id))
            }
            ResolvedTarget::Selection { criteria, .. } => {
                criteria
                    .number
                    .as_ref()
                    .is_some_and(|r| r.min.is_some() && r.min == r.max && r.min == bid.number)
                    && criteria.accepts(&bid.traits, bid.number)
            }
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests;
