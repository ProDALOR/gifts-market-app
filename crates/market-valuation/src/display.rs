//! Strategy-owned display projections. Typed calculation outputs remain authoritative.
use crate::{ValuationIssueCode, ValuationMetric};
use market_gifts::{Amount, CurrencyId};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ValuationDisplay<'a> {
    /// Ordered by the strategy; primary fields receive emphasis without changing their order.
    pub fields: Vec<DisplayField<'a>>,
    pub summary: Option<String>,
    pub note: &'static str,
}
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DisplayField<'a> {
    pub id: &'static str,
    pub label: &'static str,
    pub primary: bool,
    pub value: DisplayValue<'a>,
}
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum DisplayValue<'a> {
    /// Ready to display: precision and rounding were chosen by the strategy.
    Currency {
        amount: String,
        currency_id: &'a CurrencyId,
    },
    Number {
        amount: String,
        suffix: &'static str,
    },
    Unavailable {
        label: &'static str,
        message: &'a str,
        detail: Option<String>,
    },
}
impl<'a> DisplayField<'a> {
    pub fn currency(
        id: &'static str,
        label: &'static str,
        metric: &'a ValuationMetric,
        currency_id: &'a CurrencyId,
        decimals: Option<u8>,
    ) -> Self {
        Self::metric(id, label, metric, |amount| DisplayValue::Currency {
            amount: formatted(amount, decimals),
            currency_id,
        })
    }
    pub fn number(
        id: &'static str,
        label: &'static str,
        metric: &'a ValuationMetric,
        suffix: &'static str,
        decimals: Option<u8>,
    ) -> Self {
        Self::metric(id, label, metric, |amount| DisplayValue::Number {
            amount: formatted(amount, decimals),
            suffix,
        })
    }
    fn metric(
        id: &'static str,
        label: &'static str,
        metric: &'a ValuationMetric,
        value: impl FnOnce(&Amount) -> DisplayValue<'a>,
    ) -> Self {
        let value = match metric {
            ValuationMetric::Value { value: amount } => value(amount),
            ValuationMetric::Unavailable { error } => DisplayValue::Unavailable {
                label: match error.code {
                    ValuationIssueCode::NotEnoughData => "Not enough data",
                    ValuationIssueCode::MissingTrait => "Missing trait",
                    ValuationIssueCode::InvalidBaseline => "Invalid baseline",
                    ValuationIssueCode::CalculationOverflow => "Calculation overflow",
                },
                message: &error.message,
                detail: error.required_samples.map(|required| {
                    format!(
                        "{} / {required} required samples",
                        error.available_samples.unwrap_or(0)
                    )
                }),
            },
        };
        Self {
            id,
            label,
            primary: false,
            value,
        }
    }
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
}
fn formatted(amount: &Amount, decimals: Option<u8>) -> String {
    match decimals {
        None => amount.to_string(),
        Some(decimals) => format!(
            "{:.*}",
            usize::from(decimals),
            amount.decimal().round_dp(u32::from(decimals))
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_precision_is_exact_decimal_and_never_mutates_the_calculated_value() {
        for (input, precision, expected) in [
            (
                "10000000000000000.000000001",
                None,
                "10000000000000000.000000001",
            ),
            ("20.125", Some(2), "20.12"),
            ("20.135", Some(2), "20.14"),
            ("1.2", Some(3), "1.200"),
            ("2.5", Some(0), "2"),
        ] {
            let amount = Amount::new(input).unwrap();
            assert_eq!(formatted(&amount, precision), expected);
            assert_eq!(amount, Amount::new(input).unwrap());
        }
    }
}
