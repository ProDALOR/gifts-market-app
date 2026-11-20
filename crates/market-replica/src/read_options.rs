//! Local read policy, separate from the upstream wire contract and stored replica data.
use market_hub_protocol::wire as w;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AttributeMatch {
    /// Match concrete ask/execution values using the upstream scope semantics.
    #[default]
    Value = 0,
    /// Bid constraints accept at least one selected value per attribute.
    Overlap = 1,
    /// Bid constraints accept every selected value and cover selected ranges.
    Covers = 2,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ReadOptions {
    pub attribute_match: AttributeMatch,
}
impl ReadOptions {
    /// Empty selections have one canonical policy for validation and shared-feed identity.
    pub fn normalized_for(self, request: &w::ReadRequest) -> Self {
        if request.selection.as_ref().is_some_and(|selection| {
            matches!(&selection.source, Some(w::selection::Source::Scope(scope)) if !scope.attributes.is_empty())
        }) {
            self
        } else {
            Self::default()
        }
    }
}
