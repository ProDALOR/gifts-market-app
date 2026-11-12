use crate::{ArchiveEntry, ArchiveError, GapReason, invalid};
use market_view::Cursor;
#[derive(Clone, Default)]
pub(crate) struct State {
    pub(crate) cursor: Option<Cursor>,
    pub(crate) catalog: Option<(String, String)>,
    progress: Option<(String, String, usize)>,
    pub(crate) entries: std::sync::Arc<Vec<market_view::CatalogEntry>>,
    pending: im::Vector<market_view::CatalogEntry>,
    last_batch: Option<[u8; 32]>,
}
fn valid_cursor(cursor: &Cursor) -> Result<(), ArchiveError> {
    market_view::validate_id(&cursor.generation).map_err(|_| invalid("invalid cursor generation"))
}
impl State {
    pub(crate) fn apply(
        &mut self,
        entry: &ArchiveEntry,
        digest: [u8; 32],
    ) -> Result<bool, ArchiveError> {
        match entry {
            ArchiveEntry::CatalogPage {
                generation,
                revision,
                offset,
                entries,
                complete,
            } => {
                market_view::validate_id(generation)
                    .map_err(|_| invalid("invalid catalog generation"))?;
                market_view::validate_id(revision)
                    .map_err(|_| invalid("invalid catalog revision"))?;
                if entries.len() > 128 || (entries.is_empty() && !complete) {
                    return Err(invalid("invalid catalog page"));
                }
                if *offset != 0
                    && self.progress.as_ref()
                        != Some(&(generation.clone(), revision.clone(), *offset))
                {
                    return Err(invalid("catalog page gap"));
                }
                if *offset == 0 {
                    self.pending.clear();
                }
                self.pending.extend(entries.iter().cloned());
                if *complete {
                    self.entries = std::sync::Arc::new(self.pending.iter().cloned().collect());
                    self.pending.clear();
                    self.catalog = Some((generation.clone(), revision.clone()));
                    self.progress = None;
                } else {
                    self.progress =
                        Some((generation.clone(), revision.clone(), offset + entries.len()));
                }
            }
            ArchiveEntry::Gap {
                after,
                resumed_after,
                reason,
            } => {
                valid_cursor(resumed_after)?;
                if after != &self.cursor {
                    return Err(invalid("gap does not continue the archive cursor"));
                }
                let valid_reason = match (reason, after) {
                    (GapReason::InitialStart, None) => true,
                    (GapReason::GenerationChanged, Some(old)) => {
                        old.generation != resumed_after.generation
                    }
                    (GapReason::ReplayExpired, Some(old)) => {
                        old.generation == resumed_after.generation
                            && old.revision < resumed_after.revision
                    }
                    _ => false,
                };
                if !valid_reason {
                    return Err(invalid("gap reason does not match the boundary"));
                }
                self.cursor = Some(resumed_after.clone());
                self.last_batch = None;
            }
            ArchiveEntry::Batch {
                cursor,
                catalog_revision,
                events,
            } => {
                valid_cursor(cursor)?;
                if self.catalog.as_ref()
                    != Some(&(cursor.generation.clone(), catalog_revision.clone()))
                {
                    return Err(invalid("batch catalog has not been archived"));
                }
                if events.len() > 500 {
                    return Err(invalid("too many events"));
                }
                for event in events {
                    if !event.is_history() {
                        return Err(invalid("non-history event in archive"));
                    }
                    event
                        .validate()
                        .map_err(|_| invalid("invalid historical event"))?;
                }
                let previous = self
                    .cursor
                    .as_ref()
                    .ok_or_else(|| invalid("initial boundary required"))?;
                if cursor == previous {
                    if self.last_batch == Some(digest) {
                        return Ok(false);
                    }
                    return Err(invalid("cursor reused with different events"));
                }
                if cursor.generation != previous.generation
                    || previous.revision.checked_add(1) != Some(cursor.revision)
                {
                    return Err(invalid("batch gap requires an explicit boundary"));
                }
                self.cursor = Some(cursor.clone());
                self.last_batch = Some(digest);
            }
        }
        Ok(true)
    }
}
