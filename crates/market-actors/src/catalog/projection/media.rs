//! Bounded reads for gift media document lookup; interpretation lives in market-gifts-hub.
use super::read::entities;
use market_gifts::*;
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use std::collections::BTreeSet;
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
pub(crate) fn model_documents(
    reader: ReplicaReader,
    limit: usize,
) -> Result<Vec<TelegramDocumentId>, Status> {
    let reader = reader.snapshot()?;
    let mut documents = BTreeSet::new();
    let mut token = String::new();
    for _ in 0..250 {
        let result = reader.read(w::ReadRequest {
            selection: Some(w::Selection {
                kinds: vec![w::EntityKind::AttributeOption as i32],
                ..Default::default()
            }),
            limit: 200,
            page_token: token,
            mode: w::ReadMode::Live as i32,
            ..Default::default()
        })?;
        for row in result.page.entities {
            if let Some(w::entity::Entity::AttributeOption(option)) = row.entity
                && let Some(document) = market_gifts_hub::model_document(&option)
            {
                documents.insert(document);
                if documents.len() == limit {
                    return Ok(documents.into_iter().collect());
                }
            }
        }
        token = result.page.next_page_token;
        if token.is_empty() {
            break;
        }
    }
    Ok(documents.into_iter().collect())
}
