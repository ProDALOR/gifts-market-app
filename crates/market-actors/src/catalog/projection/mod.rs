//! Gift projections over one pinned replica view; no HTTP or lifecycle concerns.
mod choices;
mod collections;
mod markets;
mod media;
mod read;
pub(super) use choices::{ChoiceProjection, choices};
pub(super) use collections::{collections, validate_collection};
pub(super) use markets::{books, currencies, venues};
pub(super) use media::{media_document, model_documents};
pub(super) use read::entities;
#[cfg(test)]
mod tests;
