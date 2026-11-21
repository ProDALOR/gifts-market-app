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
        Ok(context
            .floor_statistics(&input.gift().collection_id)?
            .report(input.gift()))
    }
}
