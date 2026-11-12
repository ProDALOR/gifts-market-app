use super::ReplicaConfig;
use market_query::{MarketState, PreparedChanges};
use market_view::protocol::{catalog::CatalogSnapshot, wire as w};
use market_view::{Cursor, MarketEvent};
use prost::Message;
use std::sync::Arc;
use tonic::Status;

/// A complete immutable materialization. All fields belong to the same revision.
#[derive(Clone)]
pub struct ReplicaView {
    pub catalog: Arc<CatalogSnapshot>,
    pub state: MarketState,
    pub cursor: Cursor,
}
impl ReplicaView {
    pub fn record_bytes(&self) -> usize {
        self.state.total_weight()
    }
    #[cfg(test)]
    pub(super) fn apply(
        &self,
        cursor: Cursor,
        events: Vec<MarketEvent>,
        config: &ReplicaConfig,
    ) -> Result<Self, Status> {
        self.apply_batch(
            &market_view::ChangeBatch {
                cursor,
                catalog_revision: self.catalog.revision.clone(),
                catalog: Vec::new(),
                events,
            },
            config,
        )
    }
    pub(super) fn apply_batch(
        &self,
        batch: &market_view::ChangeBatch,
        config: &ReplicaConfig,
    ) -> Result<Self, Status> {
        let mut next = self.clone();
        if batch.catalog.is_empty() {
            if batch.catalog_revision != self.catalog.revision {
                return Err(Status::data_loss("missing catalog delta"));
            }
        } else {
            let catalog = Arc::new(
                market_view::protocol::catalog::apply(
                    &self.catalog,
                    batch.catalog.clone(),
                    &batch.catalog_revision,
                )
                .map_err(|_| Status::data_loss("invalid catalog delta"))?,
            );
            next.state = next
                .state
                .with_catalog(catalog.catalog.clone())
                .map_err(|_| Status::data_loss("catalog invalidates retained records"))?;
            next.catalog = catalog;
        }
        let changes = PreparedChanges::new(
            next.catalog.catalog.clone(),
            batch
                .events
                .iter()
                .filter_map(MarketEvent::mutation)
                .collect(),
            |record| w::Record::from(record.clone()).encoded_len(),
        )
        .map_err(|_| Status::data_loss("invalid delta records"))?;
        next.state
            .apply_prepared_delta(&mut next.cursor, &batch.cursor, &changes)
            .map_err(|_| Status::data_loss("delta gap or invalid records"))?;
        if next.state.len() > config.max_records || next.record_bytes() > config.max_snapshot_bytes
        {
            return Err(Status::resource_exhausted("replica capacity exceeded"));
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_view::{
        Catalog, CatalogEntry, FeedDefinition, FeedState, FeedStatus, Mutation, Record,
        VenueDefinition,
    };

    fn feed(id: &str, synced: bool) -> Record {
        Record::Feed(FeedStatus {
            feed_id: id.parse().unwrap(),
            venue_id: "venue".parse().unwrap(),
            state: if synced {
                FeedState::Ready
            } else {
                FeedState::Syncing
            },
            last_sync_at: synced.then(|| "2026-11-09T12:00:00Z".parse().unwrap()),
        })
    }
    fn initial() -> ReplicaView {
        let catalog = Arc::new(CatalogSnapshot::new(
            "generation".into(),
            Arc::new(
                Catalog::from_entries([
                    CatalogEntry::Venue(VenueDefinition {
                        id: "venue".parse().unwrap(),
                        name: "Venue".into(),
                    }),
                    CatalogEntry::Feed(FeedDefinition {
                        id: "a".parse().unwrap(),
                        venue_id: "venue".parse().unwrap(),
                    }),
                    CatalogEntry::Feed(FeedDefinition {
                        id: "b".parse().unwrap(),
                        venue_id: "venue".parse().unwrap(),
                    }),
                ])
                .unwrap(),
            ),
        ));
        let mut state = MarketState::new(catalog.catalog.clone());
        state
            .apply_prepared(
                &PreparedChanges::new(
                    catalog.catalog.clone(),
                    vec![Mutation::Upsert(feed("a", false))],
                    |record| w::Record::from(record.clone()).encoded_len(),
                )
                .unwrap(),
            )
            .unwrap();
        ReplicaView {
            catalog,
            state,
            cursor: Cursor {
                generation: "generation".into(),
                revision: 1,
            },
        }
    }
    #[test]
    fn candidate_limits_and_invalid_deltas_never_change_the_active_view() {
        let view = initial();
        let cursor = Cursor {
            generation: "generation".into(),
            revision: 2,
        };
        let update = MarketEvent::Upsert(feed("a", true));
        let mut config = ReplicaConfig::new(
            "http://127.0.0.1:50052".into(),
            market_reader::registration::NodeCredentials {
                id: "test-reader".into(),
                role: w::NodeRole::Reader,
                token: "test-reader-credential".into(),
            },
        );
        config.max_snapshot_bytes = view.record_bytes();
        assert_eq!(
            view.apply(cursor.clone(), vec![update.clone()], &config)
                .err()
                .unwrap()
                .code(),
            tonic::Code::ResourceExhausted
        );
        config.max_snapshot_bytes = 1024 * 1024;
        config.max_records = 1;
        assert_eq!(
            view.apply(
                cursor.clone(),
                vec![MarketEvent::Upsert(feed("b", false))],
                &config
            )
            .err()
            .unwrap()
            .code(),
            tonic::Code::ResourceExhausted
        );
        assert_eq!(
            view.apply(
                cursor.clone(),
                vec![update.clone(), update.clone()],
                &config
            )
            .err()
            .unwrap()
            .code(),
            tonic::Code::DataLoss
        );
        assert_eq!(view.cursor.revision, 1);
        assert_eq!(view.state.len(), 1);
        assert_eq!(
            view.state.get(&feed("a", false).key()),
            Some(&feed("a", false))
        );
        assert_eq!(
            view.record_bytes(),
            w::Record::from(feed("a", false)).encoded_len()
        );

        let empty_delta = view.apply(cursor.clone(), Vec::new(), &config).unwrap();
        assert_eq!(empty_delta.cursor, cursor);
        assert_eq!(empty_delta.record_bytes(), view.record_bytes());
        let next = view.apply(cursor, vec![update], &config).unwrap();
        assert_eq!(next.cursor.revision, 2);
        assert_eq!(
            next.record_bytes(),
            w::Record::from(feed("a", true)).encoded_len()
        );
        assert!(Arc::ptr_eq(&next.catalog, &view.catalog));
        assert_eq!(view.cursor.revision, 1);
    }
}
