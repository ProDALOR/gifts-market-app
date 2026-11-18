use crate::Priority;
use std::collections::VecDeque;
/// Prefer higher priorities, but reserve four/two/one turns per round.
/// FIFO within a class. Unready work retains its position and credit.
pub(crate) struct Queue<K> {
    entries: [VecDeque<K>; 3],
    credits: [usize; 3],
}
impl<K: PartialEq> Default for Queue<K> {
    fn default() -> Self {
        Self {
            entries: std::array::from_fn(|_| VecDeque::new()),
            credits: [4, 2, 1],
        }
    }
}
impl<K: PartialEq> Queue<K> {
    pub fn remove(&mut self, key: &K) {
        for queue in &mut self.entries {
            queue.retain(|entry| entry != key);
        }
    }
    pub fn insert(&mut self, key: K, priority: Priority) {
        if self.entries[priority.index()].contains(&key) {
            return;
        }
        self.remove(&key);
        self.entries[priority.index()].push_back(key);
    }
    pub fn pop_ready(&mut self, mut ready: impl FnMut(&K) -> bool) -> Option<K> {
        for _ in 0..2 {
            let mut eligible = false;
            for index in 0..3 {
                if let Some(position) = self.entries[index].iter().position(&mut ready) {
                    eligible = true;
                    if self.credits[index] > 0 {
                        self.credits[index] -= 1;
                        return self.entries[index].remove(position);
                    }
                }
            }
            if !eligible {
                return None;
            }
            self.credits = [4, 2, 1];
        }
        None
    }
}
