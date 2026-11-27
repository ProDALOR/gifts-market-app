//! Bounded, expiring in-memory report/share records; no persistent link store.
use std::{
    collections::{VecDeque, hash_map::RandomState},
    hash::BuildHasher,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
pub(super) struct Cache<T> {
    entries: Mutex<VecDeque<(u64, Instant, T)>>,
    next: AtomicU64,
    random: RandomState,
    prefix: &'static str,
    limit: usize,
}
impl<T> Cache<T> {
    pub fn new(prefix: &'static str, limit: usize) -> Self {
        assert!(limit > 0);
        Self {
            entries: Default::default(),
            next: Default::default(),
            random: Default::default(),
            prefix,
            limit,
        }
    }
}
impl<T: Clone> Cache<T> {
    pub fn insert(&self, view: T) -> Vec<u8> {
        self.find_or_insert(|_| false, || view).0
    }
    /// Coalesce report/share records without extending their original expiry.
    pub fn find_or_insert(
        &self,
        matches: impl Fn(&T) -> bool,
        build: impl FnOnce() -> T,
    ) -> (Vec<u8>, T) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        entries.retain(|(_, at, _)| at.elapsed() < Duration::from_secs(1800));
        let entry = if let Some(index) = entries.iter().position(|(_, _, view)| matches(view)) {
            entries.remove(index).expect("located cache entry")
        } else {
            let id = self
                .random
                .hash_one(self.next.fetch_add(1, Ordering::Relaxed));
            while entries.len() >= self.limit {
                entries.pop_front();
            }
            (id, Instant::now(), build())
        };
        let result = (
            format!("{}:{:016x}", self.prefix, entry.0).into_bytes(),
            entry.2.clone(),
        );
        entries.push_back(entry);
        result
    }
    pub fn get(&self, id: u64) -> Option<T> {
        self.update(id, |value| value.clone())
    }
    /// Change small report metadata atomically without replacing its shared data or expiry.
    pub fn update<R>(&self, id: u64, apply: impl FnOnce(&mut T) -> R) -> Option<R> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        entries.retain(|(_, at, _)| at.elapsed() < Duration::from_secs(1800));
        let index = entries.iter().position(|(key, _, _)| *key == id)?;
        let mut entry = entries.remove(index)?;
        let result = apply(&mut entry.2);
        entries.push_back(entry);
        Some(result)
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callbacks_expire_even_when_recently_accessed() {
        let views = Cache::<u64>::new("t", 2);
        let token = views.insert(1);
        let id = u64::from_str_radix(std::str::from_utf8(&token[2..]).unwrap(), 16).unwrap();
        let created = views.entries.lock().unwrap()[0].1;
        assert!(views.get(id).is_some());
        assert_eq!(views.entries.lock().unwrap()[0].1, created);
        views.update(id, |value| *value = 2);
        assert_eq!(views.entries.lock().unwrap()[0].1, created);
        assert_eq!(views.get(id), Some(2));
        views.entries.lock().unwrap()[0].1 = Instant::now() - Duration::from_secs(1801);
        assert!(views.get(id).is_none());
        assert!(views.entries.lock().unwrap().is_empty());
    }
}
