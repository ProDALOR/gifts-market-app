//! A generation, rather than the client ID alone, identifies one subscription lifetime.
use crate::error::{ApiError, Result};
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct SubscriptionId(String);
impl SubscriptionId {
    pub fn parse(value: String) -> Result<Self> {
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || b"_.-".contains(&value))
        {
            return Err(ApiError::invalid("invalid subscription ID"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Key {
    pub id: SubscriptionId,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Pending,
    Active,
    Finishing,
}
struct Entry {
    key: Key,
    cancel: CancellationToken,
    phase: Phase,
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(super) struct Registry {
    entries: HashMap<SubscriptionId, Entry>,
    generation: u64,
    limit: usize,
}
impl Registry {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
            limit,
        }
    }
    pub fn reserve(
        &mut self,
        id: SubscriptionId,
        stop: &CancellationToken,
    ) -> Result<(Key, CancellationToken)> {
        if self.entries.contains_key(&id) {
            return Err(ApiError::invalid("duplicate subscription ID"));
        }
        if self.entries.len() >= self.limit {
            return Err(ApiError::capacity("subscription limit reached"));
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| ApiError::capacity("subscription generations exhausted"))?;
        let key = Key {
            id: id.clone(),
            generation: self.generation,
        };
        let cancel = stop.child_token();
        self.entries.insert(
            id,
            Entry {
                key: key.clone(),
                cancel: cancel.clone(),
                phase: Phase::Pending,
            },
        );
        Ok((key, cancel))
    }
    pub fn current(&self, key: &Key) -> bool {
        self.entries
            .get(&key.id)
            .is_some_and(|entry| entry.key == *key)
    }
    pub fn key(&self, id: &SubscriptionId) -> Option<Key> {
        self.entries.get(id).map(|entry| entry.key.clone())
    }
    pub fn phase(&mut self, key: &Key, phase: Phase) {
        if let Some(entry) = self
            .entries
            .get_mut(&key.id)
            .filter(|entry| entry.key == *key)
            && entry.phase != Phase::Finishing
        {
            entry.phase = phase;
        }
    }
    pub fn remove(&mut self, id: &SubscriptionId) {
        self.entries.remove(id);
    }
    pub fn finish(&mut self, key: &Key) {
        if self.current(key) {
            self.remove(&key.id);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacing_a_pending_or_active_subscription_invalidates_old_frames_and_completions() {
        let mut registry = Registry::new(1);
        let stop = CancellationToken::new();
        let id = SubscriptionId::parse("offers".into()).unwrap();
        let (first, cancel) = registry.reserve(id.clone(), &stop).unwrap();
        assert!(registry.reserve(id.clone(), &stop).is_err());
        registry.remove(&id);
        assert!(cancel.is_cancelled());
        let (second, cancel) = registry.reserve(id.clone(), &stop).unwrap();
        assert!(!registry.current(&first));
        assert!(registry.current(&second));
        registry.finish(&first);
        assert!(registry.current(&second));
        registry.phase(&second, Phase::Active);
        registry.phase(&second, Phase::Finishing);
        registry.finish(&second);
        assert!(cancel.is_cancelled());
    }
}
