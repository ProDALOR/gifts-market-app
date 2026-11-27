//! Stateless share destinations, with a small cache of native prepared IDs only.
use super::*;
use market_telegram::{PreparedMessage, PreparedMessages};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::OnceCell;
mod target;
pub use target::{PortfolioTarget, ShareFallback};
pub(super) use target::{market_fallback, parse_target, portfolio_fallback, with_share};
#[derive(Debug)]
pub enum ShareError {
    Invalid(String),
    Unavailable(String),
    Busy,
}
impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(v) | Self::Unavailable(v) => f.write_str(v),
            Self::Busy => f.write_str("Sharing is busy. Please try again"),
        }
    }
}
impl std::error::Error for ShareError {}
#[derive(Clone)]
pub struct SharedMessage {
    pub prepared: Option<PreparedMessage>,
    pub fallback: ShareFallback,
}
struct Entry {
    user: u64,
    key: [u8; 32],
    created: Instant,
    prepared: OnceCell<Option<PreparedMessage>>,
}
#[derive(Default)]
struct PreparedCache(Mutex<VecDeque<Arc<Entry>>>);
impl PreparedCache {
    fn entry(&self, user: u64, key: [u8; 32], now: i64) -> Arc<Entry> {
        let mut entries = self.0.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|e| {
            e.created.elapsed() < Duration::from_secs(60)
                && e.prepared
                    .get()
                    .is_none_or(|p| p.as_ref().is_none_or(|p| p.expires_at > now + 5))
        });
        let entry = entries
            .iter()
            .position(|e| e.user == user && e.key == key)
            .and_then(|index| entries.remove(index))
            .unwrap_or_else(|| {
                Arc::new(Entry {
                    user,
                    key,
                    created: Instant::now(),
                    prepared: OnceCell::new(),
                })
            });
        while entries.len() >= 128 {
            entries.pop_front();
        }
        entries.push_back(entry.clone());
        entry
    }
}
#[derive(Clone)]
pub struct Sharing {
    commands: Arc<MarketCommands>,
    transport: PreparedMessages,
    prepared: Arc<PreparedCache>,
    slots: Arc<tokio::sync::Semaphore>,
}
impl Sharing {
    pub(crate) fn new(commands: Arc<MarketCommands>, transport: PreparedMessages) -> Self {
        Self {
            commands,
            transport,
            prepared: Default::default(),
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }
    async fn prepare<F: Future<Output = Result<Reply, ShareError>>>(
        &self,
        user: u64,
        key: String,
        fallback: ShareFallback,
        build: F,
    ) -> Result<SharedMessage, ShareError> {
        if user == 0 || user > i64::MAX as u64 || key.len() > market_links::MAX_SELECTION_BYTES {
            return Err(ShareError::Invalid("Invalid sharing target".into()));
        }
        if !self.transport.available() {
            return Ok(SharedMessage {
                prepared: None,
                fallback,
            });
        }
        let _permit = self.slots.try_acquire().map_err(|_| ShareError::Busy)?;
        let key: [u8; 32] = Sha256::digest(key.as_bytes()).into();
        let now = jiff::Timestamp::now().as_second();
        let entry = self.prepared.entry(user, key, now);
        let prepared = entry
            .prepared
            .get_or_init(|| async {
                // Fallback targets remain usable even when market reads or native preparation fail.
                let reply = build.await.ok()?;
                if reply.kind == market_telegram::ReplyKind::Notice || reply.title.is_none() {
                    return None;
                }
                let reply = target::with_share(reply, &fallback);
                tokio::time::timeout(
                    Duration::from_secs(12),
                    self.transport
                        .prepare(user, self.commands.reports.reply(user, reply.bounded())),
                )
                .await
                .ok()?
                .ok()
            })
            .await
            .clone()
            .filter(|p| p.expires_at > jiff::Timestamp::now().as_second() + 5);
        Ok(SharedMessage { prepared, fallback })
    }
    pub async fn gift(
        &self,
        user: u64,
        gift: market_gifts::GiftSlug,
        currency_id: market_gifts::CurrencyId,
    ) -> Result<SharedMessage, ShareError> {
        let currency_id = currency(&currency_id)?;
        let query = format!("{gift} {currency_id}");
        self.prepare(
            user,
            query.clone(),
            ShareFallback::Inline { query },
            async {
                Ok(self
                    .commands
                    .value(ValueRequest { gift, currency_id })
                    .await)
            },
        )
        .await
    }
    pub async fn market(
        &self,
        user: u64,
        mut selection: MarketSelection,
    ) -> Result<SharedMessage, ShareError> {
        selection
            .validate()
            .map_err(|e| ShareError::Invalid(e.to_string()))?;
        if selection.filters.collection_id.is_none() {
            return Err(ShareError::Invalid(
                "Choose a collection before sharing".into(),
            ));
        }
        selection.filters.currency_id = currency(&selection.filters.currency_id)?;
        let query = target::market_query(&selection)?;
        let fallback = target::market_fallback(&selection, self.commands.public_url.as_ref())?;
        self.prepare(user, query, fallback, async {
            Ok(self.commands.browse(selection).await)
        })
        .await
    }
    pub async fn portfolio(
        &self,
        user: u64,
        mut target: PortfolioTarget,
    ) -> Result<SharedMessage, ShareError> {
        target
            .validate()
            .map_err(|e| ShareError::Invalid(e.to_string()))?;
        target.currency_id = currency(&target.currency_id)?;
        let query = target::portfolio_query(&target)?;
        let fallback = target::portfolio_fallback(&target, self.commands.public_url.as_ref())?;
        self.prepare(user, query, fallback, async {
            self.commands.load_portfolio_target(target).await
        })
        .await
    }
}
fn currency(id: &market_gifts::CurrencyId) -> Result<market_gifts::CurrencyId, ShareError> {
    BotCurrency::parse(id.as_str())
        .map(BotCurrency::id)
        .ok_or_else(|| ShareError::Invalid("Unsupported report currency".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn prepared_ids_coalesce_per_user_target_and_evict_expired_or_excess_entries() {
        let cache = PreparedCache::default();
        let first = cache.entry(7, [1; 32], 100);
        let same = cache.entry(7, [1; 32], 100);
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &cache.entry(8, [1; 32], 100)));
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let build = || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tokio::task::yield_now().await;
            Some(PreparedMessage {
                id: "native".into(),
                expires_at: 110,
            })
        };
        let (a, b) = tokio::join!(
            first.prepared.get_or_init(build),
            same.prepared.get_or_init(build)
        );
        assert_eq!(a.as_ref().unwrap().id, b.as_ref().unwrap().id);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(
            !Arc::ptr_eq(&first, &cache.entry(7, [1; 32], 106)),
            "expired native IDs do not block a new preparation"
        );
        for i in 0..200 {
            cache.entry(7, [i; 32], 106);
        }
        assert_eq!(cache.0.lock().unwrap().len(), 128);
    }
}
