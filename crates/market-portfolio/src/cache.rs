use crate::{InventorySnapshot, UserId};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::time::Instant;
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub id: u64,
    pub inventory: Arc<InventorySnapshot>,
}
struct Entry {
    snapshot: Snapshot,
    expires: Instant,
    used: u64,
}
pub(crate) struct Cache {
    entries: BTreeMap<UserId, Entry>,
    capacity: usize,
    ttl: Duration,
    clock: u64,
}
impl Cache {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            entries: BTreeMap::new(),
            capacity,
            ttl,
            clock: 0,
        }
    }
    pub fn get(&mut self, user: UserId) -> Option<Snapshot> {
        self.entries
            .retain(|_, entry| entry.expires > Instant::now());
        let entry = self.entries.get_mut(&user)?;
        self.clock = self.clock.saturating_add(1);
        entry.used = self.clock;
        Some(entry.snapshot.clone())
    }
    pub fn insert(&mut self, user: UserId, snapshot: Snapshot) {
        if self.capacity == 0 {
            return;
        }
        self.entries
            .retain(|_, entry| entry.expires > Instant::now());
        if self.entries.len() >= self.capacity
            && !self.entries.contains_key(&user)
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key)
        {
            self.entries.remove(&oldest);
        }
        self.clock = self.clock.saturating_add(1);
        self.entries.insert(
            user,
            Entry {
                snapshot,
                expires: Instant::now() + self.ttl,
                used: self.clock,
            },
        );
    }
}
