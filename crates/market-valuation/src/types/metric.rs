//! Reusable value-or-issue metrics; strategy payloads are defined with their strategies.
use market_gifts::{Amount, InvalidValue};
use rust_decimal::Decimal;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationDependency {
    CollectionFloor,
    ModelFloor,
    BackdropFloor,
    ModelBackdropFloor,
    ModelMultiplier,
    BackdropMultiplier,
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
    /// Input that prevented the final mark; the original issue and evidence are retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency: Option<ValuationDependency>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum ValuationMetric<T = Amount> {
    Value { value: T },
    Unavailable { error: ValuationIssue },
}
impl<T> ValuationMetric<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value { value } => Some(value),
            Self::Unavailable { .. } => None,
        }
    }
}

/// A nonnegative dimensionless decimal, distinct from a monetary amount.
/// ```compile_fail
/// use market_valuation::{Multiplier, ValuationMetric};
/// let multiplier = ValuationMetric::Value { value: Multiplier::new("1.5").unwrap() };
/// let price: ValuationMetric<market_gifts::Amount> = multiplier;
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", schema(value_type = String))]
pub struct Multiplier(String);
impl Multiplier {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        Decimal::from_str_exact(&value.into())
            .map_err(|_| InvalidValue("invalid exact multiplier"))?
            .try_into()
    }
    pub fn decimal(&self) -> Decimal {
        Decimal::from_str_exact(&self.0).expect("validated multiplier")
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<Decimal> for Multiplier {
    type Error = InvalidValue;
    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        if value < Decimal::ZERO {
            return Err(InvalidValue("multiplier must not be negative"));
        }
        Ok(Self(value.normalize().to_string()))
    }
}
impl TryFrom<String> for Multiplier {
    type Error = InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl std::fmt::Display for Multiplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiplier_metrics_use_exact_validated_decimal_strings() {
        let json = serde_json::json!({"value":"10000000000000000.000000001"});
        let metric: ValuationMetric<Multiplier> = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(
            metric.value().unwrap().as_str(),
            "10000000000000000.000000001"
        );
        assert_eq!(serde_json::to_value(metric).unwrap(), json);
        for value in ["-1", "NaN", "Infinity", "bad"] {
            assert!(Multiplier::new(value).is_err());
        }
        assert!(serde_json::from_value::<Multiplier>(serde_json::json!(1.5)).is_err());
    }
}
