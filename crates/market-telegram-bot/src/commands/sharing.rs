//! On-demand, user-bound sharing over existing reports and cached portfolio pages.
use super::*;
use market_telegram::{PreparedMessage, PreparedMessages};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::OnceCell;

#[derive(Debug)]
pub enum ShareError {
    Invalid(String),
    RestartRead,
    Unavailable(String),
    Busy,
}
impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(v) | Self::Unavailable(v) => f.write_str(v),
            Self::RestartRead => f.write_str("Portfolio changed; reload the page before sharing"),
            Self::Busy => f.write_str("Sharing is busy. Please try again"),
        }
    }
}
impl std::error::Error for ShareError {}
#[derive(Clone)]
pub struct SharedMessage {
    pub prepared: Option<PreparedMessage>,
    pub inline_query: String,
    pub expires_at: i64,
}
pub(super) struct Entry {
    user: u64,
    key: [u8; 32],
    created: Instant,
    expires_at: i64,
    reply: OnceCell<Reply>,
    prepared: OnceCell<Option<PreparedMessage>>,
}
#[derive(Clone)]
pub(super) enum SharedView {
    Prepared(Arc<Entry>),
    /// An explicitly requested, already visible gallery. Public read-only snapshot; no recheck.
    Snapshot {
        reply: Arc<Reply>,
        expires_at: i64,
    },
}
#[derive(Clone)]
pub struct Sharing {
    commands: Arc<MarketCommands>,
    transport: PreparedMessages,
    slots: Arc<tokio::sync::Semaphore>,
}
impl Sharing {
    pub(crate) fn new(commands: Arc<MarketCommands>, transport: PreparedMessages) -> Self {
        Self {
            commands,
            transport,
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }
    pub(super) async fn prepare<F: Future<Output = Result<Reply, ShareError>>>(
        &self,
        user: u64,
        key: String,
        build: F,
    ) -> Result<SharedMessage, ShareError> {
        if user == 0 || user > i64::MAX as u64 || key.len() > market_links::MAX_SELECTION_BYTES {
            return Err(ShareError::Invalid("Invalid sharing target".into()));
        }
        let _permit = self.slots.try_acquire().map_err(|_| ShareError::Busy)?;
        let digest: [u8; 32] = Sha256::digest(key.as_bytes()).into();
        drop(key);
        let key = digest;
        let (token, view) = self.commands.shares.find_or_insert(
            |view| {
                matches!(view, SharedView::Prepared(entry) if entry.user == user && entry.key == key && entry.created.elapsed() < Duration::from_secs(60))
            },
            || SharedView::Prepared(Arc::new(Entry {
                    user,
                    key,
                    created: Instant::now(),
                    expires_at: jiff::Timestamp::now().as_second() + 600,
                    reply: OnceCell::new(),
                    prepared: OnceCell::new(),
                })),
        );
        let inline_query = String::from_utf8(token).expect("ASCII share token");
        let SharedView::Prepared(entry) = view else {
            unreachable!("prepared share cache entry")
        };
        let reply = entry
            .reply
            .get_or_try_init(|| async {
                let mut reply = build.await?;
                if reply.kind == market_telegram::ReplyKind::Notice || reply.title.is_none() {
                    return Err(ShareError::Unavailable(reply.text));
                }
                // Share the same snapshot through the fallback, including the original filters/page.
                for row in &mut reply.buttons {
                    for button in row {
                        if let market_telegram::ReplyButton::SwitchInline { query, .. } = button {
                            *query = inline_query.clone();
                        }
                    }
                }
                Ok(reply.bounded())
            })
            .await?;
        let prepared = entry
            .prepared
            .get_or_init(|| async {
                match tokio::time::timeout(
                    Duration::from_secs(12),
                    self.transport.prepare(
                        user,
                        self.commands.inline_controls.reply(user, reply.clone()),
                    ),
                )
                .await
                {
                    Ok(Ok(prepared)) => Some(prepared),
                    _ => None, // The exact report remains available through inline sharing.
                }
            })
            .await
            .clone()
            .filter(|p| p.expires_at > jiff::Timestamp::now().as_second() + 5);
        let expires_at = prepared
            .as_ref()
            .map_or(entry.expires_at, |p| p.expires_at.min(entry.expires_at));
        Ok(SharedMessage {
            prepared,
            inline_query,
            expires_at,
        })
    }
    pub async fn gift(
        &self,
        user: u64,
        gift: market_gifts::GiftSlug,
        currency_id: market_gifts::CurrencyId,
    ) -> Result<SharedMessage, ShareError> {
        let currency_id = currency(&currency_id)?;
        let key = format!("gift:{gift}:{currency_id}");
        self.prepare(user, key, async {
            Ok(self
                .commands
                .value(ValueRequest { gift, currency_id })
                .await)
        })
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
        let key = format!(
            "market:{}",
            serde_json::to_string(&selection).map_err(|e| ShareError::Invalid(e.to_string()))?
        );
        self.prepare(user, key, async {
            Ok(self.commands.browse(selection).await)
        })
        .await
    }
    pub async fn portfolio(
        &self,
        user: u64,
        mut query: market_portfolio::PortfolioReadQuery<
            market_actors::input::CollectionRef,
            market_actors::input::ModelRef,
            market_actors::input::BackdropRef,
            market_actors::input::PatternRef,
        >,
    ) -> Result<SharedMessage, ShareError> {
        query
            .validate()
            .map_err(|e| ShareError::Invalid(e.to_string()))?;
        query.currency_id = currency(&query.currency_id)?;
        query.limit = query.limit.min(30);
        let input = market_actors::input::GiftFilters {
            collection_id: query.filters.collection_id,
            currency_id: query.currency_id.clone(),
            models: query.filters.models,
            backdrops: query.filters.backdrops,
            patterns: query.filters.patterns,
            venue_id: None,
            number: None,
            price: None,
        };
        let filters = if input.has_aliases() {
            self.commands
                .market
                .resolve_filters(input)
                .await
                .map_err(|e| {
                    if e.code() == market_actors::error::DataErrorCode::InvalidQuery {
                        ShareError::Invalid(e.to_string())
                    } else {
                        ShareError::Unavailable(e.to_string())
                    }
                })?
        } else {
            input
                .into_resolved()
                .map_err(|e| ShareError::Invalid(e.to_string()))?
        };
        let query = market_portfolio::ResolvedPortfolioReadQuery {
            user_id: query.user_id,
            currency_id: query.currency_id,
            sort: query.sort,
            offset: query.offset,
            limit: query.limit,
            version: query.version,
            filters: market_portfolio::ResolvedPortfolioFilters {
                collection_id: filters.collection_id,
                models: filters.models,
                backdrops: filters.backdrops,
                patterns: filters.patterns,
                number: query.filters.number,
            },
        };
        let key = format!(
            "portfolio:{}",
            serde_json::to_string(&query).map_err(|e| ShareError::Invalid(e.to_string()))?
        );
        self.prepare(user, key, self.commands.shared_portfolio(query))
            .await
    }
}
fn currency(id: &market_gifts::CurrencyId) -> Result<market_gifts::CurrencyId, ShareError> {
    BotCurrency::parse(id.as_str())
        .map(BotCurrency::id)
        .ok_or_else(|| ShareError::Invalid("Unsupported report currency".into()))
}
fn snapshot_reply(reply: &Reply, token: &str) -> Reply {
    let mut reply = reply.clone();
    let mut found = false;
    for button in reply.buttons.iter_mut().flatten() {
        if let market_telegram::ReplyButton::SwitchInline { query, .. } = button {
            *query = token.to_owned();
            found = true;
        }
    }
    if !found {
        reply
            .buttons
            .push(vec![market_telegram::ReplyButton::SwitchInline {
                text: "Share".into(),
                query: token.into(),
            }]);
    }
    reply
}
impl MarketCommands {
    /// Reuse the report already rendered for this page. Inline sharing never resolves/revalues it.
    pub(super) fn share_gallery(&self, reply: Reply) -> Reply {
        if reply.kind != market_telegram::ReplyKind::Message
            || !matches!(&reply.artwork, Some(market_media::Artwork::Gallery(_)))
        {
            return reply;
        }
        let reply = reply.bounded();
        let now = jiff::Timestamp::now().as_second();
        let (token, _) = self.shares.find_or_insert(
            |view| matches!(view, SharedView::Snapshot { reply: cached, expires_at } if *expires_at > now && cached.as_ref() == &reply),
            || SharedView::Snapshot { reply: Arc::new(reply.clone()), expires_at: now + 600 },
        );
        snapshot_reply(
            &reply,
            std::str::from_utf8(&token).expect("ASCII share token"),
        )
    }
    pub(super) async fn shared_reply(&self, user: u64, id: u64) -> Reply {
        let now = jiff::Timestamp::now().as_second();
        let reply = match self.shares.get(id) {
            Some(SharedView::Prepared(entry)) if entry.user == user && entry.expires_at > now => {
                entry.reply.get().cloned()
            }
            Some(SharedView::Snapshot { reply, expires_at }) if expires_at > now => {
                Some(snapshot_reply(&reply, &format!("s:{id:016x}")))
            }
            _ => None,
        };
        reply.unwrap_or_else(|| {
            Reply::text(
                "This share expired. Open the app or request a fresh report to share again.",
            )
        })
    }
}
