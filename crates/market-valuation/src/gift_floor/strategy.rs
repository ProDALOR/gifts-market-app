use super::GiftFloorValuation;
use crate::{GiftAttributes, Strategy, ValuationContext, ValuationError};
pub struct GiftFloor;
impl Strategy for GiftFloor {
    const DISPLAY: crate::DisplayDefinition = super::display::DISPLAY;
    type Input = GiftAttributes;
    type Output = GiftFloorValuation;
    const ID: &'static str = "gift_floor";
    const NAME: &'static str = "Gift floor";
    const DESCRIPTION: &'static str = "Collection ask floor plus nonnegative model and backdrop premiums, optionally capped by matching pair asks. Symbol floors remain separate.";
    const DOCUMENTATION: &'static str = include_str!("methodology.md");
    fn evaluate(
        &self,
        input: &Self::Input,
        context: &ValuationContext<'_>,
    ) -> Result<Self::Output, ValuationError> {
        let stats = context.floor_statistics(&input.collection_id)?;
        let pair = match (input.traits.model.as_ref(), input.traits.backdrop.as_ref()) {
            (Some(model), Some(backdrop))
                if stats.known_models.contains(model)
                    && stats.known_backdrops.contains(backdrop) =>
            {
                stats.pair_floor(context.snapshot(), model, backdrop)?
            }
            _ => crate::floors::Floor::default().valuation(false),
        };
        Ok(stats.report(&input.traits, pair))
    }
}
