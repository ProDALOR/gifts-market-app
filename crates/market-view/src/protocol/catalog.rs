use crate::{Catalog, CatalogEntry, ValidationError};
use std::sync::Arc;
pub const CATALOG_PAGE_ENTRIES: usize = 128;
pub const MAX_CATALOG_ENTRIES: usize = 200_000;
pub const MAX_CATALOG_BYTES: usize = 64 * 1024 * 1024;
#[derive(Debug, Clone)]
pub struct CatalogSnapshot {
    pub generation: String,
    pub revision: String,
    pub catalog: Arc<Catalog>,
    pub entries: Arc<Vec<CatalogEntry>>,
}
impl CatalogSnapshot {
    /// Local synthetic revision for tests/materializations; received revisions are opaque.
    pub fn new(generation: String, catalog: Arc<Catalog>) -> Self {
        let revision = revision(&catalog);
        Self::received(generation, revision, catalog)
    }
    pub fn received(generation: String, revision: String, catalog: Arc<Catalog>) -> Self {
        let entries = Arc::new(catalog.entries().collect());
        Self {
            generation,
            revision,
            catalog,
            entries,
        }
    }
}
pub fn revision(catalog: &Catalog) -> String {
    use prost::Message;
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for entry in catalog.entries() {
        let bytes = super::wire::CatalogEntry::from(entry).encode_to_vec();
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("local-{:x}", hash.finalize())
}
pub fn apply(
    previous: &CatalogSnapshot,
    entries: Vec<CatalogEntry>,
    revision: &str,
) -> Result<CatalogSnapshot, ValidationError> {
    let mut all: std::collections::BTreeMap<_, _> = previous
        .catalog
        .entries()
        .map(|e| (e.identity(), e))
        .collect();
    for e in entries {
        all.insert(e.identity(), e);
    }
    let catalog = Arc::new(Catalog::from_entries(all.into_values())?);
    Ok(CatalogSnapshot::received(
        previous.generation.clone(),
        revision.into(),
        catalog,
    ))
}
