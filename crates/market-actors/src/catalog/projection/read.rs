//! Bounded catalog reads over the caller's pinned replica view.
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use tonic::Status;
pub(crate) fn invalid(value: impl std::fmt::Display) -> Status {
    Status::data_loss(value.to_string())
}
pub(crate) fn entities(
    reader: &ReplicaReader,
    kind: w::EntityKind,
    ids: Option<&[String]>,
) -> Result<Vec<w::entity::Entity>, Status> {
    if let Some(ids) = ids.filter(|ids| ids.len() > 500) {
        if ids.len() > 50_000 {
            return Err(Status::resource_exhausted(
                "gift catalog exceeds response budget",
            ));
        }
        let mut rows = Vec::new();
        for chunk in ids.chunks(500) {
            rows.extend(entities(reader, kind, Some(chunk))?);
        }
        return Ok(rows);
    }
    if ids.is_some_and(<[String]>::is_empty) {
        return Ok(vec![]);
    }
    let mut request = w::ReadRequest {
        selection: Some(w::Selection {
            kinds: vec![kind as i32],
            keys: ids
                .unwrap_or_default()
                .iter()
                .map(|id| w::Key {
                    kind: kind as i32,
                    id: id.clone(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
        mode: w::ReadMode::Live as i32,
        limit: 500,
        ..Default::default()
    };
    let mut rows = Vec::new();
    loop {
        let result = reader.read(request.clone())?;
        rows.extend(
            result
                .page
                .entities
                .into_iter()
                .filter_map(|row| row.entity),
        );
        if rows.len() > 50_000 {
            return Err(Status::resource_exhausted(
                "gift catalog exceeds response budget",
            ));
        }
        if result.page.next_page_token.is_empty() {
            return Ok(rows);
        }
        request.page_token = result.page.next_page_token;
    }
}
