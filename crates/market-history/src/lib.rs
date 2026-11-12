//! Independent binary event archive; active state never depends on archive IO.
mod format;
mod recovery;
mod writer;
use market_view::{CatalogEntry, Cursor, MarketEvent};
use serde::{Deserialize, Serialize};
use std::io;
use thiserror::Error;
pub use writer::{ArchiveOptions, HistoryArchive};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapReason {
    InitialStart,
    GenerationChanged,
    ReplayExpired,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchiveEntry {
    CatalogPage {
        generation: String,
        revision: String,
        offset: usize,
        entries: Vec<CatalogEntry>,
        complete: bool,
    },
    Gap {
        after: Option<Cursor>,
        resumed_after: Cursor,
        reason: GapReason,
    },
    Batch {
        cursor: Cursor,
        catalog_revision: String,
        events: Vec<MarketEvent>,
    },
}
#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Decode(#[from] prost::DecodeError),
    #[error("invalid history archive: {0}")]
    Invalid(&'static str),
}
pub(crate) fn invalid(message: &'static str) -> ArchiveError {
    ArchiveError::Invalid(message)
}

mod wire {
    include!(concat!(env!("OUT_DIR"), "/market_app.archive.rs"));
}
