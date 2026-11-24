//! Shared budgets for bounded replica reads, updates and retained views.
pub(crate) const MAX_APPLY_BATCHES: usize = 64;
pub(crate) const MAX_APPLY_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_PAGES: usize = 8;
pub(crate) const MAX_SNAPSHOT_ROWS: usize = 4_000;
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_HISTORY_INDEX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct ReadLimits {
    /// Maximum slice size; public actors apply this bound before read admission.
    pub max_rows: u32,
    /// Larger pages remain available to internal bounded batch readers.
    pub max_batch_rows: u32,
    pub max_scan_candidates: usize,
    /// Protobuf entity bytes plus navigation metadata, before JSON presentation.
    pub max_read_bytes: usize,
    pub max_retained_snapshots: usize,
    pub snapshot_idle_seconds: u64,
    pub max_position_bytes: usize,
    pub max_snapshot_token_bytes: usize,
}
pub const READ_LIMITS: ReadLimits = ReadLimits {
    max_rows: 200,
    max_batch_rows: 500,
    max_scan_candidates: 10_000,
    max_read_bytes: 512 * 1024,
    max_retained_snapshots: 128,
    snapshot_idle_seconds: 60,
    max_position_bytes: 8192,
    max_snapshot_token_bytes: 128,
};
