use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Clone, Default)]
pub(crate) struct Cooldown(Arc<Mutex<Option<Instant>>>);
impl Cooldown {
    pub fn remaining(&self) -> Option<u64> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .and_then(|until| until.checked_duration_since(Instant::now()))
            .map(|wait| wait.as_secs().saturating_add(1))
    }
    pub fn observe(&self, seconds: u64) {
        if let Some(until) = Instant::now().checked_add(Duration::from_secs(seconds)) {
            let mut current = self.0.lock().unwrap_or_else(|error| error.into_inner());
            *current = Some(current.map_or(until, |previous| previous.max(until)));
        }
    }
}
#[derive(Clone, Default)]
pub(crate) struct RateLimits {
    pub gifts: Cooldown,
    pub media: Cooldown,
    pub replies: Cooldown,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn later_shorter_wait_never_shortens_an_existing_cooldown() {
        let limit = Cooldown::default();
        limit.observe(7200);
        limit.observe(1);
        assert!(limit.remaining().unwrap() >= 7200);
        assert!(limit.clone().remaining().unwrap() >= 7200);
    }
}
