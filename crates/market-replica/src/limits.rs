//! Shared budgets for bounded replica reads and retained views.
#[derive(Debug, Clone, Copy)]
pub struct ReadLimits {
    /// Maximum window size and default generic page size; public actors cap pages here too.
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
