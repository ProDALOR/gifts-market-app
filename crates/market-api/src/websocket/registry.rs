//! A generation, rather than the client ID alone, identifies one operation lifetime.
use crate::error::{ApiError, Result};
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct OperationId(String);
impl OperationId {
    pub fn parse(value: String) -> Result<Self> {
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || b"_.-".contains(&value))
        {
            return Err(ApiError::invalid("invalid operation ID"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Key {
    pub id: OperationId,
    pub generation: u64,
}
struct Entry {
    key: Key,
    cancel: CancellationToken,
    request: bool,
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(super) struct Registry {
    entries: HashMap<OperationId, Entry>,
    generation: u64,
    limit: usize,
    requests: usize,
}
impl Registry {
    pub fn new(limit: usize, requests: usize) -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
            limit,
            requests,
        }
    }
    pub fn reserve(
        &mut self,
        id: OperationId,
        stop: &CancellationToken,
    ) -> Result<(Key, CancellationToken)> {
        self.reserve_kind(id, stop, false)
    }
    pub fn request(
        &mut self,
        id: OperationId,
        stop: &CancellationToken,
    ) -> Result<(Key, CancellationToken)> {
        self.reserve_kind(id, stop, true)
    }
    fn reserve_kind(
        &mut self,
        id: OperationId,
        stop: &CancellationToken,
        request: bool,
    ) -> Result<(Key, CancellationToken)> {
        if self.entries.contains_key(&id) {
            return Err(ApiError::invalid("duplicate operation ID"));
        }
        if self
            .entries
            .values()
            .filter(|entry| entry.request == request)
            .count()
            >= if request { self.requests } else { self.limit }
        {
            return Err(ApiError::capacity("operation limit reached"));
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| ApiError::capacity("operation generations exhausted"))?;
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
                request,
            },
        );
        Ok((key, cancel))
    }
    pub fn current(&self, key: &Key) -> bool {
        self.entries
            .get(&key.id)
            .is_some_and(|entry| entry.key == *key)
    }
    pub fn key(&self, id: &OperationId) -> Option<Key> {
        self.entries.get(id).map(|entry| entry.key.clone())
    }
    pub fn remove(&mut self, id: &OperationId) {
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
    fn operation_ids_reject_empty_overlong_and_non_ascii_values() {
        for value in [
            "".to_owned(),
            "x".repeat(65),
            "gift/id".into(),
            "with space".into(),
            "😀".into(),
        ] {
            assert!(OperationId::parse(value).is_err());
        }
        assert!(OperationId::parse("a".repeat(64)).is_ok());
        assert!(OperationId::parse("op_1.next-2".into()).is_ok());
    }
    #[test]
    fn requests_have_an_independent_budget_but_share_identity_and_cancellation() {
        let mut registry = Registry::new(1, 1);
        let stop = CancellationToken::new();
        let id = |s: &str| OperationId::parse(s.into()).unwrap();
        registry.reserve(id("stream"), &stop).unwrap();
        let (key, cancel) = registry.request(id("request"), &stop).unwrap();
        assert!(registry.request(id("full"), &stop).is_err());
        assert!(registry.request(id("stream"), &stop).is_err());
        registry.remove(&id("request"));
        assert!(cancel.is_cancelled());
        assert!(!registry.current(&key));
        registry.request(id("next"), &stop).unwrap();
    }
    #[test]
    fn replacing_a_pending_or_active_subscription_invalidates_old_frames_and_completions() {
        let mut registry = Registry::new(1, 1);
        let stop = CancellationToken::new();
        let id = OperationId::parse("offers".into()).unwrap();
        let (first, cancel) = registry.reserve(id.clone(), &stop).unwrap();
        assert!(registry.reserve(id.clone(), &stop).is_err());
        registry.remove(&id);
        assert!(cancel.is_cancelled());
        let (second, cancel) = registry.reserve(id.clone(), &stop).unwrap();
        assert!(!registry.current(&first));
        assert!(registry.current(&second));
        registry.finish(&first);
        assert!(registry.current(&second));
        registry.finish(&second);
        assert!(cancel.is_cancelled());
    }
}
