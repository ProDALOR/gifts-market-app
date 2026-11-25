//! Bounded metadata projection shared by catalog, finite reads and live feeds.
use market_gifts_hub::AliasCatalog;
use market_hub_protocol::wire as w;
use market_replica::{MetadataRevision, ReplicaReader};
use std::sync::{Arc, Mutex, OnceLock};
use tonic::Status;
type Cell = Arc<OnceLock<Result<Arc<AliasCatalog>, Status>>>;
#[derive(Default)]
pub(crate) struct AliasCache(Mutex<Option<(MetadataRevision, Cell)>>);
impl AliasCache {
    pub fn get(&self, reader: &ReplicaReader) -> Result<Arc<AliasCatalog>, Status> {
        let revision =
            reader.metadata_revision(&[w::EntityKind::ItemType, w::EntityKind::AttributeOption])?;
        let cell = {
            let mut cache = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if cache.as_ref().is_none_or(|(old, _)| *old != revision) {
                *cache = Some((revision, Default::default()));
            }
            cache.as_ref().unwrap().1.clone()
        };
        let result = cell
            .get_or_init(|| {
                let items =
                    market_replica::catalog_read::entities(reader, w::EntityKind::ItemType, None)?
                        .into_iter()
                        .filter_map(|v| match v {
                            w::entity::Entity::ItemType(v) => Some(v),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                let options = market_replica::catalog_read::entities(
                    reader,
                    w::EntityKind::AttributeOption,
                    None,
                )?
                .into_iter()
                .filter_map(|v| match v {
                    w::entity::Entity::AttributeOption(v) => Some(v),
                    _ => None,
                })
                .collect::<Vec<_>>();
                AliasCatalog::new_checked(&items, &options, || {
                    reader
                        .check_cancelled()
                        .map_err(|_| market_gifts::AliasError::Cancelled)
                })
                .map(Arc::new)
                .map_err(alias_status)
            })
            .clone();
        if result.is_err() {
            let mut cache = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if cache
                .as_ref()
                .is_some_and(|(_, current)| Arc::ptr_eq(current, &cell))
            {
                *cache = None;
            }
        }
        result
    }
    pub fn resolve(
        &self,
        reader: &ReplicaReader,
        query: &market_gifts_hub::input::GiftOrderQuery,
    ) -> Result<w::ReadRequest, Status> {
        let filters = if query.filters.has_aliases() {
            self.get(reader)?
                .filters(&query.filters)
                .map_err(alias_status)?
        } else {
            query
                .filters
                .clone()
                .into_resolved()
                .map_err(alias_status)?
        };
        market_gifts_hub::query::compile(&query.clone().map_filters(filters))
            .map_err(|e| Status::invalid_argument(e.to_string()))
    }
}
pub(crate) fn alias_status(error: market_gifts::AliasError) -> Status {
    match error {
        market_gifts::AliasError::Cancelled => Status::cancelled(error.to_string()),
        market_gifts::AliasError::BudgetExceeded(_) => {
            Status::resource_exhausted(error.to_string())
        }
        market_gifts::AliasError::InvalidCatalog(_) => Status::unavailable(error.to_string()),
        _ => Status::invalid_argument(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_gifts::{AliasError, GiftTraitKind};
    #[test]
    fn alias_failures_keep_cancellation_capacity_and_invalid_data_distinct() {
        for (error, expected) in [
            (AliasError::UnknownCollection, tonic::Code::InvalidArgument),
            (
                AliasError::AmbiguousTrait(GiftTraitKind::Pattern),
                tonic::Code::InvalidArgument,
            ),
            (
                AliasError::ConflictingBackdrop,
                tonic::Code::InvalidArgument,
            ),
            (
                AliasError::BudgetExceeded("bounded expansion"),
                tonic::Code::ResourceExhausted,
            ),
            (AliasError::Cancelled, tonic::Code::Cancelled),
            (
                AliasError::InvalidCatalog("invalid catalog choice"),
                tonic::Code::Unavailable,
            ),
        ] {
            assert_eq!(alias_status(error).code(), expected);
        }
    }
}
