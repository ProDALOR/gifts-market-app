//! gift_floor selects its fields and presentation from checked, typed calculation output.
use super::GiftFloorValuation;
use crate::{DisplayField, ValuationDisplay};
impl GiftFloorValuation {
    pub fn display<'a>(&'a self, currency: &'a market_gifts::CurrencyId) -> ValuationDisplay<'a> {
        ValuationDisplay {
            fields: vec![
                DisplayField::currency(
                    "final_mark",
                    "Final mark",
                    &self.final_mark,
                    currency,
                    Some(2),
                )
                .primary(),
                DisplayField::currency(
                    "collection_floor",
                    "Collection floor",
                    &self.collection_floor,
                    currency,
                    None,
                ),
                DisplayField::currency(
                    "model_floor",
                    "Model floor",
                    &self.model_floor,
                    currency,
                    None,
                ),
                DisplayField::currency(
                    "backdrop_floor",
                    "Backdrop floor",
                    &self.backdrop_floor,
                    currency,
                    None,
                ),
                DisplayField::currency(
                    "model_backdrop_floor",
                    "Model × backdrop floor",
                    &self.model_backdrop_floor,
                    currency,
                    None,
                ),
                DisplayField::number(
                    "model_multiplier",
                    "Model multiplier",
                    &self.model_multiplier,
                    "×",
                    Some(2),
                ),
                DisplayField::number(
                    "backdrop_multiplier",
                    "Backdrop multiplier",
                    &self.backdrop_multiplier,
                    "×",
                    Some(2),
                ),
            ],
            summary: Some(format!(
                "{} distinct gifts · {} collection listings",
                self.evidence.collection.gifts, self.evidence.collection.listings
            )),
            note: "Floor-based heuristic from current asks, before fees.",
        }
    }
}
