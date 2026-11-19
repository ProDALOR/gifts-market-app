//! Bounded reads for gift media document lookup; interpretation lives in market-gifts-hub.
use super::read::entities;
use market_gifts::*;
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use tonic::Status;
pub(crate) fn media_document(
    reader: ReplicaReader,
    target: MediaTarget,
) -> Result<TelegramDocumentId, Status> {
    let id = match &target {
        MediaTarget::Model(id) => id.to_string(),
        MediaTarget::Pattern(id) => id.to_string(),
    };
    let reader = reader.snapshot()?;
    entities(&reader, w::EntityKind::AttributeOption, Some(&[id]))?
        .into_iter()
        .find_map(|row| match row {
            w::entity::Entity::AttributeOption(option) => {
                market_gifts_hub::media_document(&option, &target)
            }
            _ => None,
        })
        .ok_or_else(|| Status::not_found("gift media not found"))
}
