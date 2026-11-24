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
    /// Bid constraints equal the full selection, including omitted restrictions.
    Exact = 2,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ReadOptions {
    pub attribute_match: AttributeMatch,
    /// Local ranking; omitted options use only the upstream wire sort contract.
    pub sort: Option<crate::Sort>,
}
impl ReadOptions {
    /// An empty overlap selection browses all bids; an empty exact selection matches unrestricted bids.
    pub fn normalized_for(self, request: &w::ReadRequest) -> Self {
        let has_filters = request.selection.as_ref().is_some_and(|selection| {
            matches!(&selection.source, Some(w::selection::Source::Scope(scope)) if !scope.attributes.is_empty())
        });
        if self.attribute_match == AttributeMatch::Exact || has_filters {
            self
        } else {
            Self {
                attribute_match: AttributeMatch::Value,
                ..self
            }
        }
    }
}
