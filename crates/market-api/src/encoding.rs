//! Bounded page encoding shared by subscribers to the same actor page.
use crate::{
    error::{ApiError, Result},
    models::OrderPageResponse,
};
use market_gifts::{GiftOrder, Page};
use std::{
    collections::VecDeque,
    io::{self, Write},
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{OnceCell, Semaphore};
use tokio_util::task::TaskTracker;
type NativePage = Page<GiftOrder>;
type Cell = Arc<OnceCell<Result<Arc<str>>>>;
struct Entry {
    source: Weak<NativePage>,
    encoded: Cell,
}
pub(crate) struct PageEncoder {
    cache: Mutex<VecDeque<Entry>>,
    slots: Arc<Semaphore>,
    jobs: TaskTracker,
    limit: usize,
}
impl PageEncoder {
    pub fn new(limit: usize) -> Self {
        Self {
            cache: Mutex::default(),
            slots: Arc::new(Semaphore::new(4)),
            jobs: TaskTracker::new(),
            limit,
        }
    }
    pub async fn encode(&self, page: Arc<NativePage>) -> Result<Arc<str>> {
        let source = Arc::downgrade(&page);
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
                .map_err(|_| ApiError::unavailable("Page encoding is unavailable"))?;
            let limit = self.limit;
            self.jobs
                .spawn_blocking(move || {
                    let _permit = permit;
                    encode(&page, limit)
                })
                .await
                .map_err(|_| ApiError::unavailable("Page encoding failed"))?
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
            return Err(io::Error::other("page limit exceeded"));
        }
        self.bytes.extend_from_slice(value);
        Ok(value.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode(page: &NativePage, limit: usize) -> Result<Arc<str>> {
    let mut writer = Limited {
        bytes: Vec::new(),
        limit,
        overflow: false,
    };
    if serde_json::to_writer(&mut writer, &OrderPageResponse::from(page)).is_err() {
        return Err(if writer.overflow {
            ApiError::capacity("Subscription page is too large; reduce the page limit")
        } else {
            ApiError::unavailable("Page encoding failed")
        });
    }
    String::from_utf8(writer.bytes)
        .map(Arc::from)
        .map_err(|_| ApiError::unavailable("Page encoding failed"))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn page() -> NativePage {
        Page {
            data: vec![],
            next_page_token: "token".into(),
            cursor: "cursor".into(),
            sync: market_gifts::SyncStatus {
                ready: true,
                state: market_gifts::SyncState::Live,
                updated_at: None,
            },
        }
    }
    #[tokio::test]
    async fn subscribers_share_encoded_pages_without_changing_the_public_contract() {
        let encoder = PageEncoder::new(1024);
        let page = Arc::new(page());
        let (a, b) = tokio::join!(encoder.encode(page.clone()), encoder.encode(page.clone()));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&a).unwrap(),
            serde_json::to_value(OrderPageResponse::from(&*page)).unwrap()
        );
        encoder.shutdown().await;
    }
    #[tokio::test]
    async fn oversized_pages_have_an_explicit_terminal_error() {
        let encoder = PageEncoder::new(8);
        assert!(matches!(
            encoder.encode(Arc::new(page())).await.unwrap_err().code,
            crate::error::ErrorCode::CapacityExceeded
        ));
        encoder.shutdown().await;
    }
}
