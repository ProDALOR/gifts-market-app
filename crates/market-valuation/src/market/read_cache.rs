//! Share completed reads between identical active consumers without retaining their data.
use super::{PreparedQuery, ValuationError, ValuationRead};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, Weak},
};

const CAPACITY: usize = 32;
#[derive(Default)]
pub(super) struct ReadCache(Mutex<VecDeque<(PreparedQuery, Weak<ValuationRead>)>>);
impl ReadCache {
    pub fn get(&self, key: &PreparedQuery) -> Result<Option<Arc<ValuationRead>>, ValuationError> {
        let mut entries = self
            .0
            .lock()
            .map_err(|_| ValuationError::unavailable("Valuation cache unavailable"))?;
        entries.retain(|(_, read)| read.strong_count() > 0);
        let Some(index) = entries
            .iter()
            .position(|(candidate, _)| candidate.same_query(key))
        else {
            return Ok(None);
        };
        let entry = entries.remove(index).expect("located entry");
        let read = entry.1.upgrade();
        entries.push_back(entry);
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
        entries.retain(|(candidate, read)| !candidate.same_query(&key) && read.strong_count() > 0);
        if entries.len() == CAPACITY {
            entries.pop_front();
        }
        entries.push_back((key, Arc::downgrade(read)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::read_set::ReadSet;

    #[test]
    fn weak_entries_are_bounded_and_do_not_retain_reads() {
        let cache = ReadCache::default();
        let read = Arc::new(ValuationRead {
            report: Arc::new(crate::ValuationReport {
                target: crate::ResolvedTarget::Collection {
                    collection_id: "gift".into(),
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
                target: crate::ValuationTarget::Collection {
                    collection_id: market_gifts::CollectionId::new(index.to_string()).unwrap(),
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
        assert_eq!(cache.0.lock().unwrap().len(), CAPACITY);
        assert!(cache.get(&query(0)).unwrap().is_none());
        assert!(Arc::ptr_eq(&read, &cache.get(&query(1)).unwrap().unwrap()));
        drop(read);
        assert!(cache.get(&query(1)).unwrap().is_none());
        assert!(cache.0.lock().unwrap().is_empty());
    }
}
