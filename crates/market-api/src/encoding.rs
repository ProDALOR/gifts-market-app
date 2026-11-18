//! Bounded window encoding shared by subscribers to the same immutable window.
use crate::{
    error::{ApiError, Result},
    orders::OrderWindowResponse,
};
use std::{
    collections::VecDeque,
    io::{self, Write},
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{OnceCell, Semaphore};
use tokio_util::task::TaskTracker;
type NativeWindow = market_gifts::OrderWindow;
type Cell = Arc<OnceCell<Result<Arc<str>>>>;
struct Entry {
    source: Weak<NativeWindow>,
    encoded: Cell,
}
pub(crate) struct WindowEncoder {
    cache: Mutex<VecDeque<Entry>>,
    slots: Arc<Semaphore>,
    jobs: TaskTracker,
    limit: usize,
}
impl WindowEncoder {
    pub fn new(limit: usize) -> Self {
        Self {
            cache: Mutex::default(),
            slots: Arc::new(Semaphore::new(4)),
            jobs: TaskTracker::new(),
            limit,
        }
    }
    pub async fn encode_window(&self, window: Arc<NativeWindow>) -> Result<Arc<str>> {
        let source = Arc::downgrade(&window);
        let cell = {
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            cache.retain(|entry| entry.source.strong_count() > 0);
            if let Some(entry) = cache.iter().find(|entry| entry.source.ptr_eq(&source)) {
                entry.encoded.clone()
            } else {
                let encoded = Arc::new(OnceCell::new());
                while cache.len() >= 8 {
                    cache.pop_front();
                }
                cache.push_back(Entry {
                    source,
                    encoded: encoded.clone(),
                });
                encoded
            }
        };
        cell.get_or_init(|| async {
            let permit = self
                .slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| ApiError::unavailable("Window encoding is unavailable"))?;
            let limit = self.limit;
            self.jobs
                .spawn_blocking(move || {
                    let _permit = permit;
                    encode(&window, limit)
                })
                .await
                .map_err(|_| ApiError::unavailable("Window encoding failed"))?
        })
        .await
        .clone()
    }
    pub async fn shutdown(&self) {
        self.jobs.close();
        self.jobs.wait().await;
    }
}
struct Limited {
    bytes: Vec<u8>,
    limit: usize,
    overflow: bool,
}
impl Write for Limited {
    fn write(&mut self, value: &[u8]) -> io::Result<usize> {
        if value.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.overflow = true;
            return Err(io::Error::other("window limit exceeded"));
        }
        self.bytes.extend_from_slice(value);
        Ok(value.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode(window: &NativeWindow, limit: usize) -> Result<Arc<str>> {
    let mut writer = Limited {
        bytes: Vec::new(),
        limit,
        overflow: false,
    };
    let result = serde_json::to_writer(&mut writer, &OrderWindowResponse::from(window));
    if result.is_err() {
        return Err(if writer.overflow {
            ApiError::capacity("Subscription data is too large; reduce the window size")
        } else {
            ApiError::unavailable("Window encoding failed")
        });
    }
    String::from_utf8(writer.bytes)
        .map(Arc::from)
        .map_err(|_| ApiError::unavailable("Window encoding failed"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn window_encodings_are_shared_and_bounded_by_the_same_budget() {
        let encoder = WindowEncoder::new(1024);
        let window = Arc::new(market_gifts::OrderWindow {
            data: vec![],
            anchor: None,
            has_before: false,
            has_after: false,
            cursor: "c".into(),
        });
        let (a, b) = tokio::join!(
            encoder.encode_window(window.clone()),
            encoder.encode_window(window.clone())
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&a).unwrap(),
            serde_json::to_value(crate::orders::OrderWindowResponse::from(&*window)).unwrap()
        );
        let small = WindowEncoder::new(8);
        assert!(small.encode_window(window).await.is_err());
        small.shutdown().await;
        encoder.shutdown().await;
    }
}
