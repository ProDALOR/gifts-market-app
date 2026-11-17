//! Shared ranking semantics for active indexes and durable execution indexes.
use crate::{state::decimal, types::Sort};
use market_hub_protocol::wire as w;
use rust_decimal::Decimal;
use tonic::Status;
impl Sort {
    pub fn is_price(self) -> bool {
        matches!(self, Self::PriceAsc | Self::PriceDesc)
    }
    pub fn descending(self) -> bool {
        matches!(self, Self::PriceDesc | Self::TimeDesc | Self::NumberDesc)
    }
    pub fn criterion(self) -> u8 {
        match self {
            Self::TimeAsc | Self::TimeDesc => 1,
            Self::NumberAsc | Self::NumberDesc => 2,
            _ => 0,
        }
    }
    pub fn rank(self, order: &w::Order, history: bool) -> Result<Decimal, Status> {
        Ok(match self.criterion() {
            1 => Decimal::from(if history {
                order.executed_at_ms.unwrap_or(order.observed_at_ms)
            } else {
                order.observed_at_ms
            }),
            2 => {
                use w::{attribute_predicate::Predicate as P, attribute_value::Value as V};
                match order
                    .attributes
                    .get("tg.number")
                    .and_then(|p| p.predicate.as_ref())
                {
                    Some(P::Eq(value)) => match value.value.as_ref() {
                        Some(V::Integer(number)) => Decimal::from(*number),
                        _ => Decimal::MAX,
                    },
                    _ => Decimal::MAX,
                }
            }
            _ => decimal(&order.price)?,
        })
    }
}
// Fixed-width signed decimal ordering without conversion to floating point.
// Decimal has at most 29 integral and 28 fractional digits.
pub(crate) fn ordered_decimal(value: Decimal) -> Vec<u8> {
    let negative = value.is_sign_negative();
    let text = value.abs().normalize().to_string();
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let mut bytes = vec![b'0'; 58];
    bytes[0] = if negative { 0 } else { 1 };
    bytes[30 - whole.len()..30].copy_from_slice(whole.as_bytes());
    bytes[30..30 + fraction.len()].copy_from_slice(fraction.as_bytes());
    if negative {
        for digit in &mut bytes[1..] {
            *digit = b'9' - (*digit - b'0');
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durable_decimal_keys_preserve_signed_exact_order_and_scale_equivalence() {
        let values = [
            "-10000000000000000.000000002",
            "-2",
            "-0.000000001",
            "0",
            "0.00",
            "0.0000000000000000000000000001",
            "2",
            "10",
            "10000000000000000.000000001",
            "10000000000000000.000000002",
            "79228162514264337593543950335",
        ];
        let keys: Vec<_> = values
            .into_iter()
            .map(|value| ordered_decimal(decimal(value).unwrap()))
            .collect();
        assert!(keys.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(keys[3], keys[4]);
    }
}
