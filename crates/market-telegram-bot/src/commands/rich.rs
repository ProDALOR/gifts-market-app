//! Small, escaped rich-message primitives shared by report renderers.
use crate::icons::escape;
use market_gifts::Amount;
pub(super) const COLUMNS: usize = 3;
pub(super) const CALLBACK_PREFIX: &str = "<tg-button type=\"callback_data\" data=\"";

pub(super) fn amount(value: &Amount) -> String {
    let value = value
        .decimal()
        .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
        .normalize()
        .to_string();
    let (integer, fractional) = value.split_once('.').unwrap_or((&value, ""));
    let mut result = String::with_capacity(value.len() + integer.len() / 3);
    for (i, digit) in integer.chars().enumerate() {
        if i > 0 && (integer.len() - i).is_multiple_of(3) {
            result.push(' ');
        }
        result.push(digit);
    }
    if !fractional.is_empty() {
        result.push('.');
        result.push_str(fractional);
    }
    result
}

pub(super) fn button(label: &str, data: Option<&str>, selected: bool) -> String {
    let style = if selected { " style=\"primary\"" } else { "" };
    match data {
        Some(data) => {
            debug_assert!(!data.is_empty() && data.len() <= 64);
            format!(
                "{CALLBACK_PREFIX}{}\"{style}>{}</tg-button>",
                escape(data),
                escape(label)
            )
        }
        None => format!(
            "<tg-button type=\"disabled\"{style}>{}</tg-button>",
            escape(label)
        ),
    }
}

pub(super) fn price(image: &str, amount: &str, currency: &str, url: Option<&str>) -> String {
    let content = format!("{image} <b>{}</b> {currency}", escape(amount));
    match url {
        Some(url) => format!("<a href=\"{}\">{content}</a>", escape(url)),
        None => content,
    }
}

pub(super) fn grid(cells: &[String], columns: usize) -> String {
    assert!((1..=3).contains(&columns));
    let mut html = String::from("<table striped compact>");
    for row in cells.chunks(columns) {
        html.push_str("<tr>");
        for cell in row {
            html.push_str(&format!("<td>{cell}</td>"));
        }
        for _ in row.len()..columns {
            html.push_str("<td></td>");
        }
        html.push_str("</tr>");
    }
    html.push_str("</table>");
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prices_match_the_shared_gui_fixture() {
        let cases: Vec<(String, String)> =
            serde_json::from_str(include_str!("../../../../fixtures/price-format.json")).unwrap();
        for (input, expected) in cases {
            assert_eq!(amount(&Amount::new(input).unwrap()), expected);
        }
    }
    #[test]
    fn grid_layouts_fill_exactly_two_or_three_columns() {
        for columns in [2, 3] {
            for count in [1, 5, 6, 12, 30] {
                let html = grid(&vec!["gift".into(); count], columns);
                assert_eq!(html.matches("<tr>").count(), count.div_ceil(columns));
                assert_eq!(
                    html.matches("<td>").count(),
                    count.div_ceil(columns) * columns
                );
            }
        }
    }
}
