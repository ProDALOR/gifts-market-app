//! Sequential continuations, one bounded prefetch batch and one overflow page.
use crate::limits::{MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_PAGES, MAX_SNAPSHOT_ROWS};
use market_hub_client::HubClient;
use market_hub_protocol::wire as w;
use prost::Message;
use std::time::Duration;
use tonic::Status;

struct BufferedPage {
    page: w::Page,
    bytes: usize,
}

impl BufferedPage {
    fn new(page: w::Page) -> Result<Self, Status> {
        let bytes = page.encoded_len();
        if bytes > MAX_SNAPSHOT_BYTES || page.entities.len() > MAX_SNAPSHOT_ROWS {
            return Err(Status::resource_exhausted(
                "snapshot page exceeds batch budget",
            ));
        }
        Ok(Self { page, bytes })
    }
}

#[derive(Default)]
pub(super) struct SnapshotBatch {
    pub pages: Vec<w::Page>,
    pub rows: usize,
    bytes: usize,
}
impl SnapshotBatch {
    pub fn first(page: w::Page) -> Result<Self, Status> {
        let mut batch = Self::default();
        batch.push(BufferedPage::new(page)?);
        Ok(batch)
    }
    fn fits(&self, page: &BufferedPage) -> bool {
        self.pages.len() < MAX_SNAPSHOT_PAGES
            && page.page.entities.len() <= MAX_SNAPSHOT_ROWS - self.rows
            && page.bytes <= MAX_SNAPSHOT_BYTES - self.bytes
    }
    fn push(&mut self, page: BufferedPage) {
        debug_assert!(self.fits(&page));
        self.rows += page.page.entities.len();
        self.bytes += page.bytes;
        self.pages.push(page.page);
    }
    fn full(&self) -> bool {
        self.pages.len() == MAX_SNAPSHOT_PAGES
            || self.rows == MAX_SNAPSHOT_ROWS
            || self.bytes == MAX_SNAPSHOT_BYTES
    }
    pub fn into_changes(self) -> Vec<w::Change> {
        let mut changes = Vec::with_capacity(self.rows);
        changes.extend(
            self.pages
                .into_iter()
                .flat_map(|page| page.entities)
                .map(|entity| w::Change {
                    change: Some(w::change::Change::Upsert(entity)),
                }),
        );
        changes
    }
}

pub(super) struct SnapshotPages<'a> {
    hub: &'a HubClient,
    baseline: String,
    token: String,
    pending: Option<BufferedPage>,
    batch: SnapshotBatch,
    pub download: Duration,
}
impl<'a> SnapshotPages<'a> {
    pub fn new(hub: &'a HubClient, baseline: String, token: String) -> Self {
        Self {
            hub,
            baseline,
            token,
            pending: None,
            batch: SnapshotBatch::default(),
            download: Duration::ZERO,
        }
    }
    pub async fn next_batch(&mut self) -> Result<Option<SnapshotBatch>, Status> {
        // Retain the prefix and continuation across awaits. Cancelling a read
        // cannot consume its token or make a partial batch look like EOF.
        while !self.batch.full() {
            let mut page = if let Some(page) = self.pending.take() {
                page
            } else if self.token.is_empty() {
                break;
            } else {
                let (page, elapsed) = super::snapshot_page(self.hub, self.token.clone()).await?;
                self.download += elapsed;
                BufferedPage::new(page)?
            };
            if page.page.cursor != self.baseline {
                return Err(Status::data_loss("snapshot checkpoint changed"));
            }
            if !self.batch.pages.is_empty() && !self.batch.fits(&page) {
                self.pending = Some(page);
                break;
            }
            self.token = std::mem::take(&mut page.page.next_page_token);
            self.batch.push(page);
        }
        let batch = std::mem::take(&mut self.batch);
        Ok((!batch.pages.is_empty()).then_some(batch))
    }
}

#[cfg(test)]
mod tests;
