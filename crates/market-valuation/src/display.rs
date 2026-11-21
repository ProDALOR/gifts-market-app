//! Static strategy presentation contracts; query outputs contain calculation data only.
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DisplayDefinition {
    pub fields: &'static [FieldDefinition],
    pub evidence: &'static [EvidenceDefinition],
    pub note: &'static str,
}
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct FieldDefinition {
    /// Top-level metric key in the strategy's typed data, containing value or error.
    pub id: &'static str,
    pub label: &'static str,
    pub primary: bool,
    pub format: FieldFormat,
}
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum FieldFormat {
    Currency {
        decimals: Option<u8>,
    },
    Number {
        decimals: Option<u8>,
        suffix: &'static str,
    },
}
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct EvidenceDefinition {
    /// RFC 6901 JSON pointer within strategy data. Absent evidence is omitted.
    pub path: &'static str,
    pub label: &'static str,
}
impl FieldDefinition {
    pub const fn currency(id: &'static str, label: &'static str, decimals: Option<u8>) -> Self {
        Self {
            id,
            label,
            primary: false,
            format: FieldFormat::Currency { decimals },
        }
    }
    pub const fn number(
        id: &'static str,
        label: &'static str,
        suffix: &'static str,
        decimals: Option<u8>,
    ) -> Self {
        Self {
            id,
            label,
            primary: false,
            format: FieldFormat::Number { decimals, suffix },
        }
    }
    pub const fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
}

/// Local rendering for text consumers, using the same definitions advertised by discovery.
/// These projections are never added to HTTP or WebSocket results.
#[derive(Debug)]
pub struct ValuationDisplay {
    pub fields: Vec<DisplayField>,
    pub summary: Option<String>,
    pub note: &'static str,
}
#[derive(Debug)]
pub struct DisplayField {
    pub id: &'static str,
    pub label: &'static str,
    pub primary: bool,
    pub value: DisplayValue,
}
#[derive(Debug)]
pub enum DisplayValue {
    Currency {
        amount: String,
        currency_id: market_gifts::CurrencyId,
    },
    Number {
        amount: String,
        suffix: &'static str,
    },
    Unavailable {
        label: &'static str,
        message: String,
        detail: Option<String>,
    },
}
impl DisplayDefinition {
    pub(crate) fn render(
        &self,
        data: &serde_json::Value,
        error: Option<&str>,
        currency: &market_gifts::CurrencyId,
    ) -> ValuationDisplay {
        let error_field = self
            .fields
            .iter()
            .find(|field| field.primary)
            .or(self.fields.first())
            .copied()
            .unwrap_or(FieldDefinition::currency("value", "Value", None).primary());
        let definitions = if error.is_some() {
            std::slice::from_ref(&error_field)
        } else {
            self.fields
        };
        let fields = definitions
            .iter()
            .map(|field| {
                let metric = &data[field.id];
                let value = metric["value"]
                    .as_str()
                    .and_then(|value| rust_decimal::Decimal::from_str_exact(value).ok());
                let value = match (value, field.format, error) {
                    (Some(amount), FieldFormat::Currency { decimals }, None) => {
                        DisplayValue::Currency {
                            amount: formatted(amount, decimals),
                            currency_id: currency.clone(),
                        }
                    }
                    (Some(amount), FieldFormat::Number { decimals, suffix }, None) => {
                        DisplayValue::Number {
                            amount: formatted(amount, decimals),
                            suffix,
                        }
                    }
                    _ => {
                        let issue = &metric["error"];
                        DisplayValue::Unavailable {
                            label: issue_label(issue["code"].as_str().unwrap_or("")),
                            message: error
                                .or_else(|| issue["message"].as_str())
                                .unwrap_or("Value is unavailable")
                                .to_owned(),
                            detail: issue["required_samples"].as_u64().map(|required| {
                                format!(
                                    "{} / {required} required samples",
                                    issue["available_samples"].as_u64().unwrap_or(0)
                                )
                            }),
                        }
                    }
                };
                DisplayField {
                    id: field.id,
                    label: field.label,
                    primary: field.primary,
                    value,
                }
            })
            .collect();
        let evidence = if error.is_some() {
            vec![]
        } else {
            self.evidence
                .iter()
                .filter_map(|field| {
                    let value = data.pointer(field.path)?;
                    let value = value
                        .as_u64()
                        .map(|v| v.to_string())
                        .or_else(|| value.as_str().map(str::to_owned))?;
                    Some(format!("{value} {}", field.label))
                })
                .collect::<Vec<_>>()
        };
        ValuationDisplay {
            fields,
            summary: (!evidence.is_empty()).then(|| evidence.join(" · ")),
            note: self.note,
        }
    }
}
fn issue_label(code: &str) -> &'static str {
    match code {
        "not_enough_data" => "Not enough data",
        "missing_trait" => "Missing trait",
        "invalid_baseline" => "Invalid baseline",
        "calculation_overflow" => "Calculation overflow",
        _ => "Unavailable",
    }
}
fn formatted(amount: rust_decimal::Decimal, decimals: Option<u8>) -> String {
    match decimals {
        None => amount.to_string(),
        Some(decimals) => format!(
            "{:.*}",
            usize::from(decimals),
            amount.round_dp(u32::from(decimals))
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precision_is_exact_decimal_and_uses_midpoint_to_even() {
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
            assert_eq!(
                formatted(
                    rust_decimal::Decimal::from_str_exact(input).unwrap(),
                    precision
                ),
                expected
            );
        }
    }
}
