//! Independent archive consumer; all durable event I/O is outside core.
use market_history::{ArchiveEntry, GapReason, HistoryArchive};
use market_view::protocol::catalog::{self, CatalogSnapshot};
use market_view::{ChangeBatch, Cursor};
use std::sync::Arc;
pub fn archive_catalog(
    archive: &mut HistoryArchive,
    catalog: &CatalogSnapshot,
) -> anyhow::Result<()> {
    if archive.catalog() == Some((&catalog.generation, &catalog.revision)) {
        return Ok(());
    }
    let mut offset = 0;
    loop {
        let mut entries = Vec::new();
        let mut bytes = 0;
        while let Some(entry) = catalog.entries.get(offset + entries.len()) {
            let n = serde_json::to_vec(entry)?.len();
            if !entries.is_empty()
                && (entries.len() == 128
                    || bytes + n > market_view::protocol::MAX_MESSAGE_BYTES / 2)
            {
                break;
            }
            anyhow::ensure!(
                n <= market_view::protocol::MAX_MESSAGE_BYTES / 2,
                "catalog entry exceeds archive budget"
            );
            bytes += n;
            entries.push(entry.clone());
        }
        let next = offset + entries.len();
        let complete = next == catalog.entries.len();
        archive.append(ArchiveEntry::CatalogPage {
            generation: catalog.generation.clone(),
            revision: catalog.revision.clone(),
            offset,
            entries,
            complete,
        })?;
        if complete {
            break;
        }
        offset = next;
    }
    Ok(())
}
pub fn boundary(
    archive: &mut HistoryArchive,
    catalog: &CatalogSnapshot,
    cursor: Cursor,
) -> anyhow::Result<()> {
    archive_catalog(archive, catalog)?;
    if archive.cursor() == Some(&cursor) {
        return Ok(());
    }
    let reason = match archive.cursor() {
        None => GapReason::InitialStart,
        Some(previous) if previous.generation != cursor.generation => GapReason::GenerationChanged,
        Some(_) => GapReason::ReplayExpired,
    };
    archive.append(ArchiveEntry::Gap {
        after: archive.cursor().cloned(),
        resumed_after: cursor,
        reason,
    })?;
    Ok(())
}
pub fn batch(
    archive: &mut HistoryArchive,
    catalog: &mut Arc<CatalogSnapshot>,
    batch: ChangeBatch,
) -> anyhow::Result<()> {
    if !batch.catalog.is_empty() {
        *catalog = Arc::new(catalog::apply(
            catalog,
            batch.catalog,
            &batch.catalog_revision,
        )?);
        archive_catalog(archive, catalog)?;
    }
    anyhow::ensure!(
        catalog.revision == batch.catalog_revision && catalog.generation == batch.cursor.generation,
        "batch/catalog identity mismatch"
    );
    for event in &batch.events {
        anyhow::ensure!(event.is_history(), "non-history event on history stream");
        catalog.catalog.validate_event(event)?;
    }
    archive.append(ArchiveEntry::Batch {
        cursor: batch.cursor,
        catalog_revision: batch.catalog_revision,
        events: batch.events,
    })?;
    Ok(())
}
pub async fn follow(
    node: &market_reader::node::NodeConfig,
    archive: &mut HistoryArchive,
    reset: bool,
) -> anyhow::Result<()> {
    use market_reader::sync::{Limits, Reader, Update};
    let channel = node.connect().await?;
    let mut catalog = archive.catalog_snapshot()?;
    let after = if reset {
        None
    } else {
        archive.cursor().cloned()
    };
    let mut reader = Reader::connect(
        channel,
        &node.authorization()?,
        after,
        market_view::protocol::wire::ChangeProjection::History,
        Limits::default(),
    )
    .await?;
    loop {
        let update = reader
            .message()
            .await?
            .ok_or_else(|| tonic::Status::unavailable("history stream closed"))?;
        match update {
            Update::Snapshot(snapshot) => {
                boundary(archive, &snapshot.catalog, snapshot.cursor)?;
                catalog = Some(snapshot.catalog);
            }
            Update::Delta(update) => batch(
                archive,
                catalog
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("history catalog missing"))?,
                update,
            )?,
            Update::Checkpoint(cursor) | Update::Heartbeat(cursor) => anyhow::ensure!(
                archive.cursor() == Some(&cursor),
                "archive checkpoint mismatch"
            ),
        }
    }
}
