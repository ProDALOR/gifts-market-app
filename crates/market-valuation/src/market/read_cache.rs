//! Weak sharing for active watches plus a bounded LRU of completed reads.
use super::{PreparedQuery, ValuationError, ValuationRead};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, Weak},
};

const SHARING_CAPACITY: usize = 32;
#[derive(Default)]
pub(super) struct ReadCache(Mutex<Entries>);
#[derive(Default)]
struct Entries {
    weak: VecDeque<(PreparedQuery, Weak<ValuationRead>)>,
    completed: VecDeque<Arc<ValuationRead>>,
    gates: Vec<(PreparedQuery, Weak<tokio::sync::Mutex<()>>)>,
}
const COMPLETED_CAPACITY: usize = 8;
impl Entries {
    fn remove_unused(&mut self) {
        self.weak.retain(|(_, read)| read.strong_count() > 0);
    }
    fn touch(&mut self, read: &Arc<ValuationRead>) {
        self.completed.retain(|cached| !Arc::ptr_eq(cached, read));
        if self.completed.len() >= COMPLETED_CAPACITY {
            self.completed.pop_front();
        }
        self.completed.push_back(read.clone());
    }
}
impl ReadCache {
    /// Coalesce equal queries before read admission and snapshot pinning. Distinct
    /// reports can prepare concurrently without duplicating or rolling back results.
    pub fn gate(&self, key: &PreparedQuery) -> Result<Arc<tokio::sync::Mutex<()>>, ValuationError> {
        let mut entries = self
            .0
            .lock()
            .map_err(|_| ValuationError::unavailable("Valuation cache unavailable"))?;
        entries.gates.retain(|(_, gate)| gate.strong_count() > 0);
        if let Some(gate) = entries
            .gates
            .iter()
            .find(|(candidate, _)| candidate.same_query(key))
            .and_then(|(_, gate)| gate.upgrade())
        {
            return Ok(gate);
        }
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        entries.gates.push((key.clone(), Arc::downgrade(&gate)));
        Ok(gate)
    }

    pub fn get(&self, key: &PreparedQuery) -> Result<Option<Arc<ValuationRead>>, ValuationError> {
        let mut entries = self
            .0
            .lock()
            .map_err(|_| ValuationError::unavailable("Valuation cache unavailable"))?;
        entries.remove_unused();
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
        if let Some(read) = &read {
            entries.touch(read);
        }
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
        entries.remove_unused();
        let replaced = entries
            .weak
            .iter()
            .find(|(candidate, _)| candidate.same_query(&key))
            .and_then(|(_, previous)| previous.upgrade());
        entries
            .weak
            .retain(|(candidate, read)| !candidate.same_query(&key) && read.strong_count() > 0);
        if entries.weak.len() == SHARING_CAPACITY {
            entries.weak.pop_front();
        }
        entries.weak.push_back((key, Arc::downgrade(read)));
        if let Some(previous) = replaced {
            entries
                .completed
                .retain(|cached| !Arc::ptr_eq(&previous, cached));
        }
        entries.touch(read);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::read_set::ReadSet;

    fn query(index: usize) -> PreparedQuery {
        crate::ValuationQuery {
            target: crate::ValuationTarget::Selection {
                collection_id: market_gifts_hub::input::CollectionRef::new(index.to_string())
                    .unwrap(),
                criteria: Default::default(),
            },
            currency_id: "GRAM".into(),
            strategies: vec![crate::StrategyId::new("ask_floor").unwrap()],
            period: None,
        }
        .prepare()
        .unwrap()
    }
    fn read(index: usize) -> Arc<ValuationRead> {
        Arc::new(ValuationRead {
            report: Arc::new(crate::ValuationReport {
                target: crate::ResolvedTarget::Selection {
                    collection_id: market_gifts::CollectionId::new(index.to_string()).unwrap(),
                    criteria: Default::default(),
                },
                currency_id: "GRAM".into(),
                evaluated_at: jiff::Timestamp::now(),
                stale: false,
                results: vec![],
            }),
            reads: ReadSet::new(),
            external_gift: None,
        })
    }
    #[tokio::test(start_paused = true)]
    async fn retained_reports_do_not_expire_but_keep_their_freshness_deadline() {
        let cache = ReadCache::default();
        let report = read(0);
        let deadline = report.deadline();
        let weak = Arc::downgrade(&report);
        cache.insert(query(0), &report).unwrap();
        drop(report);
        tokio::time::advance(std::time::Duration::from_secs(24 * 60 * 60)).await;
        let retained = cache.get(&query(0)).unwrap().unwrap();
        assert!(Arc::ptr_eq(&retained, &weak.upgrade().unwrap()));
        assert_eq!(retained.deadline(), deadline);
        assert!(
            retained.deadline() < tokio::time::Instant::now(),
            "Retention never renews freshness"
        );
    }
    #[test]
    fn completed_reports_evict_by_recency_and_hits_promote_them() {
        let cache = ReadCache::default();
        for index in 0..COMPLETED_CAPACITY {
            cache.insert(query(index), &read(index)).unwrap();
        }
        let first = cache.get(&query(0)).unwrap().unwrap();
        let weak = Arc::downgrade(&first);
        drop(first);
        cache
            .insert(query(COMPLETED_CAPACITY), &read(COMPLETED_CAPACITY))
            .unwrap();
        assert!(
            cache.get(&query(1)).unwrap().is_none(),
            "The least recently used report is evicted"
        );
        assert!(weak.upgrade().is_some(), "A hit protects the older report");
        assert_eq!(cache.0.lock().unwrap().completed.len(), COMPLETED_CAPACITY);
    }
    #[test]
    fn active_watches_share_reads_after_completed_report_eviction() {
        let cache = ReadCache::default();
        let active = read(0); // The watch owns the read independently of the LRU.
        cache.insert(query(0), &active).unwrap();
        for index in 1..=COMPLETED_CAPACITY {
            cache.insert(query(index), &read(index)).unwrap();
        }
        assert!(
            !cache
                .0
                .lock()
                .unwrap()
                .completed
                .iter()
                .any(|read| Arc::ptr_eq(read, &active))
        );
        assert!(Arc::ptr_eq(
            &active,
            &cache.get(&query(0)).unwrap().unwrap()
        ));
        assert_eq!(cache.0.lock().unwrap().completed.len(), COMPLETED_CAPACITY);
    }
    #[test]
    fn replacing_a_query_releases_its_previous_completed_read() {
        let cache = ReadCache::default();
        let previous = read(0);
        let weak = Arc::downgrade(&previous);
        cache.insert(query(0), &previous).unwrap();
        drop(previous);
        let current = read(0);
        cache.insert(query(0), &current).unwrap();
        assert!(weak.upgrade().is_none());
        assert!(Arc::ptr_eq(
            &current,
            &cache.get(&query(0)).unwrap().unwrap()
        ));
        assert_eq!(cache.0.lock().unwrap().completed.len(), 1);
    }
    #[test]
    fn both_indexes_are_bounded_and_evicted_unused_reads_are_released() {
        let cache = ReadCache::default();
        let active: Vec<_> = (0..=SHARING_CAPACITY).map(read).collect();
        for (index, read) in active.iter().enumerate() {
            cache.insert(query(index), read).unwrap();
        }
        assert_eq!(cache.0.lock().unwrap().weak.len(), SHARING_CAPACITY);
        assert_eq!(cache.0.lock().unwrap().completed.len(), COMPLETED_CAPACITY);
        assert!(cache.get(&query(0)).unwrap().is_none());
        assert!(Arc::ptr_eq(
            &active[1],
            &cache.get(&query(1)).unwrap().unwrap()
        ));
        let oldest = Arc::downgrade(&active[2]);
        drop(active);
        assert!(oldest.upgrade().is_none());
        assert!(cache.get(&query(2)).unwrap().is_none());
        assert_eq!(cache.0.lock().unwrap().weak.len(), COMPLETED_CAPACITY);
    }
}
