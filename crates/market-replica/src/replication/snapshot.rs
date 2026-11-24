//! Sequential continuations, one bounded prefetch batch and one overflow page.
use crate::limits::{MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_PAGES, MAX_SNAPSHOT_ROWS};
use market_hub_client::Snapshot;
use market_hub_protocol::wire as w;
use prost::Message;
use std::time::Duration;
use tonic::Status;

#[derive(Clone, Copy, Default)]
pub(super) struct SnapshotStats {
    pub bytes: u64,
    pub catalog: u64,
    pub feeds: u64,
    pub asks: u64,
    pub bids: u64,
    pub executions: u64,
}
impl SnapshotStats {
    pub fn add(&mut self, other: Self) {
        self.bytes += other.bytes;
        self.catalog += other.catalog;
        self.feeds += other.feeds;
        self.asks += other.asks;
        self.bids += other.bids;
        self.executions += other.executions;
    }
}

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
    pub stats: SnapshotStats,
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
        self.stats.bytes += page.bytes as u64;
        for entity in &page.page.entities {
            match entity.entity.as_ref() {
                Some(w::entity::Entity::Order(order)) => match w::OrderMode::try_from(order.mode) {
                    Ok(w::OrderMode::Ask) => self.stats.asks += 1,
                    Ok(w::OrderMode::Bid) => self.stats.bids += 1,
                    Ok(w::OrderMode::Executed) => self.stats.executions += 1,
                    _ => {} // Invalid rows are rejected by staging, never counted as successful work.
                },
                Some(w::entity::Entity::Feed(_)) => self.stats.feeds += 1,
                Some(_) => self.stats.catalog += 1,
                None => {}
            }
        }
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

pub(super) struct SnapshotPages {
    source: Snapshot,
    baseline: String,
    pending: Option<BufferedPage>,
    batch: SnapshotBatch,
    pub download: Duration,
    failed: Option<Status>,
}
impl SnapshotPages {
    pub fn new(source: Snapshot, baseline: String) -> Self {
        Self {
            source,
            baseline,
            pending: None,
            batch: SnapshotBatch::default(),
            download: Duration::ZERO,
            failed: None,
        }
    }
    pub async fn next_batch(&mut self) -> Result<Option<SnapshotBatch>, Status> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        let result = self.read_batch().await;
        if let Err(error) = &result {
            self.failed = Some(error.clone());
        }
        result
    }
    async fn read_batch(&mut self) -> Result<Option<SnapshotBatch>, Status> {
        // Retain the prefix and continuation across awaits. Cancelling a read
        // cannot consume its token or make a partial batch look like EOF.
        while !self.batch.full() {
            let page = if let Some(page) = self.pending.take() {
                page
            } else {
                let started = std::time::Instant::now();
                let page = self.source.next().await?;
                self.download += started.elapsed();
                let Some(page) = page else { break };
                BufferedPage::new(page)?
            };
            if page.page.cursor != self.baseline {
                return Err(Status::data_loss("snapshot checkpoint changed"));
            }
            if !self.batch.pages.is_empty() && !self.batch.fits(&page) {
                self.pending = Some(page);
                break;
            }
            self.batch.push(page);
        }
        let batch = std::mem::take(&mut self.batch);
        Ok((!batch.pages.is_empty()).then_some(batch))
    }
}

#[cfg(test)]
mod tests;
