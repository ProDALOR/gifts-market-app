//! Weak watch sharing plus eight hot completed reads with a fifteen-second idle TTL.
use super::{PreparedQuery, ValuationError, ValuationRead};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, Weak},
};

const CAPACITY: usize = 32;
#[derive(Default)]
pub(super) struct ReadCache(Mutex<Entries>);
#[derive(Default)]
struct Entries {
    weak: VecDeque<(PreparedQuery, Weak<ValuationRead>)>,
    hot: VecDeque<(tokio::time::Instant, Arc<ValuationRead>)>,
}
const HOT_CAPACITY: usize = 8;
const HOT_TTL: std::time::Duration = std::time::Duration::from_secs(15);
impl Entries {
    fn prune(&mut self) {
        self.hot
            .retain(|(until, _)| *until > tokio::time::Instant::now());
        self.weak.retain(|(_, read)| read.strong_count() > 0);
    }
}
impl ReadCache {
    pub fn prune(&self) {
        if let Ok(mut entries) = self.0.try_lock() {
            entries.prune();
        }
    }
    pub fn get(&self, key: &PreparedQuery) -> Result<Option<Arc<ValuationRead>>, ValuationError> {
        let mut entries = self
            .0
            .lock()
            .map_err(|_| ValuationError::unavailable("Valuation cache unavailable"))?;
        entries.prune();
        let Some(index) = entries
            .weak
            .iter()
            .position(|(candidate, _)| candidate.same_query(key))
        else {
            return Ok(None);
        };
        let entry = entries.weak.remove(index).expect("located entry");
        let read = entry.1.upgrade();
        entries.weak.push_back(entry);
        Ok(read)
    }
    pub fn insert(
        &self,
        key: PreparedQuery,
        read: &Arc<ValuationRead>,
    ) -> Result<(), ValuationError> {
        let mut entries = self
            .0
            .lock()
            .map_err(|_| ValuationError::unavailable("Valuation cache unavailable"))?;
        entries.prune();
        let replaced = entries
            .weak
            .iter()
            .find(|(candidate, _)| candidate.same_query(&key))
            .and_then(|(_, previous)| previous.upgrade());
        entries
            .weak
            .retain(|(candidate, read)| !candidate.same_query(&key) && read.strong_count() > 0);
        if entries.weak.len() == CAPACITY {
            entries.weak.pop_front();
        }
        entries.weak.push_back((key, Arc::downgrade(read)));
        entries.hot.retain(|(_, cached)| {
            !Arc::ptr_eq(cached, read)
                && replaced
                    .as_ref()
                    .is_none_or(|previous| !Arc::ptr_eq(previous, cached))
        });
        while entries.hot.len() >= HOT_CAPACITY {
            entries.hot.pop_front();
        }
        entries.hot.push_back((
            (tokio::time::Instant::now() + HOT_TTL).min(read.deadline()),
            read.clone(),
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::read_set::ReadSet;

    #[tokio::test(start_paused = true)]
    async fn replacing_a_query_releases_its_previous_hot_read() {
        let cache = ReadCache::default();
        let query = crate::ValuationQuery {
            target: crate::ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: Default::default(),
            },
            currency_id: "GRAM".into(),
            strategies: vec![crate::StrategyId::new("ask_floor").unwrap()],
            period: None,
        }
        .prepare()
        .unwrap();
        let read = || {
            Arc::new(ValuationRead {
                report: Arc::new(crate::ValuationReport {
                    target: crate::ResolvedTarget::Selection {
                        collection_id: "gift".into(),
                        criteria: Default::default(),
                    },
                    currency_id: "GRAM".into(),
                    evaluated_at: jiff::Timestamp::now(),
                    stale: false,
                    results: vec![],
                }),
                reads: ReadSet::new(),
            })
        };
        let previous = read();
        let weak = Arc::downgrade(&previous);
        cache.insert(query.clone(), &previous).unwrap();
        drop(previous);
        assert!(weak.upgrade().is_some());
        let current = read();
        cache.insert(query.clone(), &current).unwrap();
        assert!(
            weak.upgrade().is_none(),
            "A superseded read has no cache owner"
        );
        assert!(Arc::ptr_eq(&current, &cache.get(&query).unwrap().unwrap()));
        assert_eq!(cache.0.lock().unwrap().hot.len(), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn entries_are_bounded_and_completed_reads_expire() {
        let cache = ReadCache::default();
        let read = Arc::new(ValuationRead {
            report: Arc::new(crate::ValuationReport {
                target: crate::ResolvedTarget::Selection {
                    collection_id: "gift".into(),
                    criteria: market_gifts::GiftCriteria::default(),
                },
                currency_id: "GRAM".into(),
                evaluated_at: jiff::Timestamp::now(),
                stale: false,
                results: vec![],
            }),
            reads: ReadSet::new(),
        });
        let query = |index: usize| {
            crate::ValuationQuery {
                target: crate::ValuationTarget::Selection {
                    collection_id: market_gifts::CollectionId::new(index.to_string()).unwrap(),
                    criteria: market_gifts::GiftCriteria::default(),
                },
                currency_id: "GRAM".into(),
                strategies: vec![crate::StrategyId::new("ask_floor").unwrap()],
                period: None,
            }
            .prepare()
            .unwrap()
        };
        for index in 0..=CAPACITY {
            cache.insert(query(index), &read).unwrap();
        }
        assert_eq!(cache.0.lock().unwrap().weak.len(), CAPACITY);
        assert_eq!(
            cache.0.lock().unwrap().hot.len(),
            1,
            "Repeated consumers retain one hot read"
        );
        assert!(cache.get(&query(0)).unwrap().is_none());
        assert!(Arc::ptr_eq(&read, &cache.get(&query(1)).unwrap().unwrap()));
        drop(read);
        tokio::time::advance(HOT_TTL).await;
        assert!(cache.get(&query(1)).unwrap().is_none());
        assert!(cache.0.lock().unwrap().weak.is_empty());
    }
}
