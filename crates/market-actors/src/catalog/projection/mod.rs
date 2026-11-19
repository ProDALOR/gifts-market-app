//! Gift projections over one pinned replica view; no HTTP or lifecycle concerns.
mod choices;
mod collections;
mod markets;
mod media;
mod read;
pub(super) use choices::{ChoiceProjection, choices};
pub(super) use collections::collections;
pub(super) use markets::{books, currencies, venues};
pub(super) use media::media_document;
#[cfg(test)]
mod tests;
