//! Reusable value-or-issue metrics; strategy payloads are defined with their strategies.
use market_gifts::Amount;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationIssueCode {
    NotEnoughData,
    MissingTrait,
    InvalidBaseline,
    CalculationOverflow,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationIssue {
    pub code: ValuationIssueCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_samples: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_samples: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationMetric {
    Value { value: Amount },
    Unavailable { error: ValuationIssue },
}
impl ValuationMetric {
    pub fn value(&self) -> Option<&Amount> {
        match self {
            Self::Value { value } => Some(value),
            Self::Unavailable { .. } => None,
        }
    }
}
