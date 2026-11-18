//! Bounded catalog reads over the caller's pinned replica view.
use crate::ReplicaReader;
use market_hub_protocol::wire as w;
use tonic::Status;
pub const MAX_CATALOG_ROWS: usize = 50_000;
/// A traversal may inspect unrelated rows; returned-row limits alone do not bound that work.
pub struct ScanBudget {
    pages: usize,
}
impl Default for ScanBudget {
    fn default() -> Self {
        Self::new(2560)
    }
}
impl ScanBudget {
    pub fn new(pages: usize) -> Self {
        Self { pages }
    }
    pub fn page(&mut self) -> Result<(), Status> {
        self.pages = self
            .pages
            .checked_sub(1)
            .ok_or_else(|| Status::resource_exhausted("read scan budget exceeded"))?;
        Ok(())
    }
}

pub fn entities(
    reader: &ReplicaReader,
    kind: w::EntityKind,
    ids: Option<&[String]>,
) -> Result<Vec<w::entity::Entity>, Status> {
    entities_cancellable(reader, kind, ids, reader.cancellation_token())
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
    entities_with_budget(reader, kind, ids, cancelled, &mut ScanBudget::default())
}
fn entities_with_budget(
    reader: &ReplicaReader,
    kind: w::EntityKind,
    ids: Option<&[String]>,
    cancelled: &tokio_util::sync::CancellationToken,
    budget: &mut ScanBudget,
) -> Result<Vec<w::entity::Entity>, Status> {
    check(cancelled)?;
    if let Some(ids) = ids.filter(|ids| ids.len() > 500) {
        if ids.len() > MAX_CATALOG_ROWS {
            return Err(Status::resource_exhausted(
                "catalog exceeds response budget",
            ));
        }
        let mut rows = Vec::new();
        for chunk in ids.chunks(500) {
            rows.extend(entities_with_budget(
                reader,
                kind,
                Some(chunk),
                cancelled,
                budget,
            )?);
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
        budget.page()?;
        let result = reader.read(request.clone())?;
        rows.extend(
            result
                .page
                .entities
                .into_iter()
                .filter_map(|row| row.entity),
        );
        if rows.len() > MAX_CATALOG_ROWS {
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
    trait_options_with_budget(
        reader,
        attribute,
        ids,
        cancelled,
        &mut ScanBudget::default(),
    )
}
fn trait_options_with_budget(
    reader: &ReplicaReader,
    attribute: &str,
    ids: Option<&[String]>,
    cancelled: &tokio_util::sync::CancellationToken,
    budget: &mut ScanBudget,
) -> Result<Vec<w::AttributeOption>, Status> {
    if let Some(ids) = ids {
        return Ok(entities_with_budget(
            reader,
            w::EntityKind::AttributeOption,
            Some(ids),
            cancelled,
            budget,
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
        budget.page()?;
        let page = reader.read(query.clone())?.page;
        for entity in page.entities {
            if let Some(w::entity::Entity::AttributeOption(option)) = entity.entity
                && option.attribute_id == attribute
            {
                options.push(option);
            }
        }
        if options.len() > MAX_CATALOG_ROWS {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ReplicaWriter,
        test_support::{catalog, entity, upsert},
    };
    #[test]
    fn scans_count_unmatched_pages_and_share_the_budget_between_key_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = ReplicaWriter::open(dir.path(), "catalog-budget").unwrap();
        let mut snapshot = writer.begin_snapshot().unwrap();
        snapshot.append(&catalog()).unwrap();
        let ids = (0..1001)
            .map(|i| format!("option.{i:04}"))
            .collect::<Vec<_>>();
        let options = ids
            .iter()
            .map(|id| {
                upsert(entity(w::entity::Entity::AttributeOption(
                    w::AttributeOption {
                        id: id.clone(),
                        attribute_id: "unrelated".into(),
                        name: id.clone(),
                        ..Default::default()
                    },
                )))
            })
            .collect::<Vec<_>>();
        snapshot.append(&options).unwrap();
        writer
            .install_snapshot(snapshot.finish("catalog").unwrap())
            .unwrap();
        let reader = writer.reader().snapshot().unwrap();
        let cancelled = tokio_util::sync::CancellationToken::new();
        let error =
            trait_options_with_budget(&reader, "target", None, &cancelled, &mut ScanBudget::new(1))
                .unwrap_err();
        assert_eq!(error.code(), tonic::Code::ResourceExhausted);
        assert!(
            trait_options(&reader, "target", None, &cancelled)
                .unwrap()
                .is_empty()
        );
        let error = entities_with_budget(
            &reader,
            w::EntityKind::AttributeOption,
            Some(&ids),
            &cancelled,
            &mut ScanBudget::new(1),
        )
        .unwrap_err();
        assert_eq!(error.code(), tonic::Code::ResourceExhausted);
        assert_eq!(
            entities(&reader, w::EntityKind::AttributeOption, Some(&ids))
                .unwrap()
                .len(),
            ids.len()
        );
    }
}
