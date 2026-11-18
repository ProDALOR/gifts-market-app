//! Bounded catalog reads over the caller's pinned replica view.
use crate::ReplicaReader;
use market_hub_protocol::wire as w;
use tonic::Status;
pub fn entities(
    reader: &ReplicaReader,
    kind: w::EntityKind,
    ids: Option<&[String]>,
) -> Result<Vec<w::entity::Entity>, Status> {
    entities_cancellable(
        reader,
        kind,
        ids,
        &tokio_util::sync::CancellationToken::new(),
    )
}
pub fn check(cancelled: &tokio_util::sync::CancellationToken) -> Result<(), Status> {
    if cancelled.is_cancelled() {
        Err(Status::cancelled("Market read cancelled"))
    } else {
        Ok(())
    }
}
pub fn entities_cancellable(
    reader: &ReplicaReader,
    kind: w::EntityKind,
    ids: Option<&[String]>,
    cancelled: &tokio_util::sync::CancellationToken,
) -> Result<Vec<w::entity::Entity>, Status> {
    check(cancelled)?;
    if let Some(ids) = ids.filter(|ids| ids.len() > 500) {
        if ids.len() > 50_000 {
            return Err(Status::resource_exhausted(
                "catalog exceeds response budget",
            ));
        }
        let mut rows = Vec::new();
        for chunk in ids.chunks(500) {
            rows.extend(entities_cancellable(reader, kind, Some(chunk), cancelled)?);
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
        check(cancelled)?;
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
                "catalog exceeds response budget",
            ));
        }
        if result.page.next_page_token.is_empty() {
            return Ok(rows);
        }
        request.page_token = result.page.next_page_token;
    }
}

/// Restricted traits use exact keys. Unrestricted traits retain only matching options.
pub fn trait_options(
    reader: &ReplicaReader,
    attribute: &str,
    ids: Option<&[String]>,
    cancelled: &tokio_util::sync::CancellationToken,
) -> Result<Vec<w::AttributeOption>, Status> {
    if let Some(ids) = ids {
        return Ok(entities_cancellable(
            reader,
            w::EntityKind::AttributeOption,
            Some(ids),
            cancelled,
        )?
        .into_iter()
        .filter_map(|entity| match entity {
            w::entity::Entity::AttributeOption(option) if option.attribute_id == attribute => {
                Some(option)
            }
            _ => None,
        })
        .collect());
    }
    let mut query = w::ReadRequest {
        selection: Some(w::Selection {
            kinds: vec![w::EntityKind::AttributeOption as i32],
            ..Default::default()
        }),
        limit: 500,
        mode: w::ReadMode::Live as i32,
        ..Default::default()
    };
    let mut options = Vec::new();
    loop {
        check(cancelled)?;
        let page = reader.read(query.clone())?.page;
        for entity in page.entities {
            if let Some(w::entity::Entity::AttributeOption(option)) = entity.entity
                && option.attribute_id == attribute
            {
                options.push(option);
            }
        }
        if options.len() > 50_000 {
            return Err(Status::resource_exhausted(
                "trait catalog exceeds response budget",
            ));
        }
        if page.next_page_token.is_empty() {
            return Ok(options);
        }
        query.page_token = page.next_page_token;
    }
}
