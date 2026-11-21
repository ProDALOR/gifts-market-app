use super::GiftFloorValuation;
use crate::{ResolvedGiftInput, Strategy, ValuationContext, ValuationError};
pub struct GiftFloor;
impl Strategy for GiftFloor {
    const DISPLAY: crate::DisplayDefinition = super::display::DISPLAY;
    type Input = ResolvedGiftInput;
    type Output = GiftFloorValuation;
    const ID: &'static str = "gift_floor";
    const NAME: &'static str = "Gift floor";
    const DESCRIPTION: &'static str = "A floor-based heuristic using collection and trait floors, capped by the matching model-and-backdrop floor.";
    const DOCUMENTATION: &'static str = include_str!("methodology.md");
    fn evaluate(
        &self,
        input: &Self::Input,
        context: &ValuationContext<'_>,
    ) -> Result<Self::Output, ValuationError> {
        let stats = context.floor_statistics(&input.gift().collection_id)?;
        let pair = match (
            input.gift().traits.model.as_ref(),
            input.gift().traits.backdrop.as_ref(),
        ) {
            (Some(model), Some(backdrop))
                if stats.known_models.contains(model)
                    && stats.known_backdrops.contains(backdrop) =>
            {
                crate::floors::Floor::from_valuation(&context.pair_floor(
                    &input.gift().collection_id,
                    model,
                    backdrop,
                )?)
            }
            _ => crate::floors::Floor::default(),
        };
        Ok(stats.report(input.gift(), &pair))
    }
}
