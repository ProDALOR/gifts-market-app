//! Read-only user-session gift lookup. Exported keys are imported in memory;
//! no login, message sending or modification of the export is performed.
use super::invocations::{CallError, Executor, Group};
use crate::BotError;
use grammers_client::{
    Client,
    sender::{ConnectionParams, NoRetries, SenderPool, SenderPoolHandle},
    tl,
};
use grammers_session::{Session, storages::MemorySession};
use market_gifts::{
    GiftNumber, GiftResolutionError as Error, GiftResolver, GiftSlug, TelegramDocumentId,
    TelegramGiftDetails, TelegramGiftId,
};
use market_invocations::{Priority, Replay};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    net::SocketAddr,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize, Serialize)]
pub(super) struct Export {
    pub(super) api_id: i32,
    pub(super) auth_key: String,
    pub(super) dc_id: i32,
    pub(super) server_address: SocketAddr,
}
struct Account {
    api_id: i32,
    auth_key: [u8; 256],
    dc_id: i32,
    server_address: SocketAddr,
}
impl TryFrom<Export> for Account {
    type Error = BotError;
    fn try_from(value: Export) -> Result<Self, BotError> {
        if value.api_id <= 0 || !(1..=5).contains(&value.dc_id) || value.server_address.port() == 0
        {
            return Err(BotError::Configuration(
                "Invalid Telegram account API ID, DC or address",
            ));
        }
        let mut auth_key = [0; 256];
        if value.auth_key.len() != 512 || !value.auth_key.is_ascii() {
            return Err(BotError::Configuration(
                "Telegram account auth_key must contain 512 hexadecimal characters",
            ));
        }
        for (i, byte) in auth_key.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value.auth_key[2 * i..2 * i + 2], 16)
                .map_err(|_| BotError::Configuration("Invalid Telegram account auth_key"))?;
        }
        if auth_key.iter().all(|v| *v == 0) {
            return Err(BotError::Configuration("Empty Telegram account auth_key"));
        }
        Ok(Self {
            api_id: value.api_id,
            auth_key,
            dc_id: value.dc_id,
            server_address: value.server_address,
        })
    }
}
type Cached = Result<TelegramGiftDetails, Error>;
type CacheSlot = Arc<tokio::sync::Mutex<Option<(Instant, Cached)>>>;
struct Live {
    client: Client,
    executor: Executor,
}
struct Shared {
    live: RwLock<Vec<Option<Arc<Live>>>>,
    next: AtomicUsize,
    cache: Mutex<VecDeque<(GiftSlug, CacheSlot)>>,
}
#[derive(Clone)]
pub struct AccountGiftResolver(Arc<Shared>);
pub struct AccountWorker {
    accounts: Vec<Account>,
    directory: PathBuf,
    resolver: AccountGiftResolver,
}
pub(super) fn validate_exports(bytes: &[u8]) -> Result<(), BotError> {
    parse_accounts(bytes).map(|_| ())
}
fn parse_accounts(bytes: &[u8]) -> Result<Vec<Account>, BotError> {
    if bytes.len() > 65_536 {
        return Err(BotError::Configuration(
            "Telegram accounts file exceeds 64 KiB",
        ));
    }
    let exports: Vec<Export> = serde_json::from_slice(bytes).map_err(|_| {
        BotError::Configuration(
            "Invalid Telegram accounts JSON; expected api_id, auth_key, dc_id and server_address",
        )
    })?;
    if exports.is_empty() || exports.len() > 16 {
        return Err(BotError::Configuration(
            "Configure 1..=16 Telegram user sessions",
        ));
    }
    let accounts = exports
        .into_iter()
        .map(Account::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let mut keys = std::collections::HashSet::new();
    if accounts
        .iter()
        .any(|a| !keys.insert(Sha256::digest(a.auth_key)))
    {
        return Err(BotError::Configuration(
            "Duplicate Telegram account session",
        ));
    }
    Ok(accounts)
}
impl AccountWorker {
    pub async fn from_file(path: &Path, directory: PathBuf) -> Result<Self, BotError> {
        use tokio::io::AsyncReadExt;
        let mut bytes = Vec::new();
        tokio::fs::File::open(path)
            .await
            .map_err(|_| BotError::Configuration("Cannot read Telegram accounts file"))?
            .take(65_537)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| BotError::Session)?;
        let accounts = parse_accounts(&bytes)?;
        let resolver = AccountGiftResolver(Arc::new(Shared {
            live: RwLock::new(vec![None; accounts.len()]),
            next: AtomicUsize::new(0),
            cache: Mutex::new(VecDeque::new()),
        }));
        Ok(Self {
            accounts,
            directory,
            resolver,
        })
    }
    pub fn resolver(&self) -> AccountGiftResolver {
        self.resolver.clone()
    }
    pub async fn run(self, stop: CancellationToken) -> Result<(), BotError> {
        if stop.is_cancelled() {
            return Ok(());
        }
        let scheduler = market_invocations::Runtime::start(Default::default())
            .map_err(|_| BotError::Invocations)?;
        let local = stop.child_token();
        let mut tasks = tokio::task::JoinSet::new();
        for (index, account) in self.accounts.into_iter().enumerate() {
            tasks.spawn(run_account(
                account,
                self.directory.clone(),
                self.resolver.clone(),
                index,
                scheduler.handle(),
                local.clone(),
            ));
        }
        let mut failed = tokio::select! {
            _ = stop.cancelled() => false,
            result = tasks.join_next() => result.is_some_and(|result| result.is_err()) || !stop.is_cancelled(),
        };
        local.cancel();
        while let Some(result) = tasks.join_next().await {
            failed |= result.is_err();
        }
        self.resolver.0.live.write().unwrap().fill(None);
        scheduler.shutdown().await;
        if failed {
            Err(BotError::Runner)
        } else {
            Ok(())
        }
    }
}
impl GiftResolver for AccountGiftResolver {
    fn resolve<'a>(
        &'a self,
        slug: &'a GiftSlug,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Cached> + Send + 'a>> {
        Box::pin(async move {
            let slot = {
                let mut cache = self.0.cache.lock().unwrap();
                let slot = cache
                    .iter()
                    .position(|(key, _)| key == slug)
                    .and_then(|i| cache.remove(i))
                    .map(|(_, slot)| slot)
                    .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(None)));
                cache.push_back((slug.clone(), slot.clone()));
                while cache.len() > 128 {
                    cache.pop_front();
                }
                slot
            };
            let mut cached = slot.lock().await;
            if let Some((expires, value)) = &*cached
                && *expires > Instant::now()
            {
                return value.clone();
            }
            let live = {
                let live = self.0.live.read().unwrap();
                let start = self.0.next.fetch_add(1, Ordering::Relaxed);
                (0..live.len())
                    .filter_map(|i| live[(start.wrapping_add(i)) % live.len()].as_ref())
                    .find(|live| !live.executor.stopped.is_cancelled())
                    .cloned()
                    .ok_or(Error::Unavailable)?
            };
            let client = live.client.clone();
            let requested = slug.to_string();
            let result = live
                .executor
                .run(
                    Group::GiftLookup,
                    Priority::Interactive,
                    Replay::Safe,
                    move || {
                        let client = client.clone();
                        let slug = requested.clone();
                        async move {
                            client
                                .invoke(&tl::functions::payments::GetUniqueStarGift { slug })
                                .await
                        }
                    },
                )
                .await
                .map_err(lookup_error)
                .and_then(|response| {
                    let tl::enums::payments::UniqueStarGift::Gift(response) = response;
                    map_gift(response.gift, slug)
                });
            if result.is_ok() || result == Err(Error::NotFound) {
                let ttl = if result.is_ok() { 300 } else { 15 };
                *cached = Some((Instant::now() + Duration::from_secs(ttl), result.clone()));
            }
            result
        })
    }
}
fn lookup_error(error: CallError) -> Error {
    if matches!(&error, CallError::Remote(grammers_client::sender::InvocationError::Rpc(e)) if e.name == "STARGIFT_SLUG_INVALID")
    {
        return Error::NotFound;
    }
    match error.bot() {
        BotError::Authorization => Error::Authorization,
        BotError::RetryAfter(seconds) => Error::RateLimited(seconds),
        _ => Error::Unavailable,
    }
}
fn map_gift(gift: tl::enums::StarGift, slug: &GiftSlug) -> Cached {
    let tl::enums::StarGift::Unique(gift) = gift else {
        return Err(Error::InvalidData);
    };
    let resolved = GiftSlug::new(gift.slug).map_err(|_| Error::InvalidData)?;
    let number = u32::try_from(gift.num)
        .ok()
        .and_then(|n| GiftNumber::new(n).ok())
        .ok_or(Error::InvalidData)?;
    if gift.burned {
        return Err(Error::NotFound);
    }
    if gift.id == 0 || &resolved != slug || number != slug.number() {
        return Err(Error::InvalidData);
    }
    let mut model = None;
    let mut backdrop = None;
    let mut pattern = None;
    for attr in gift.attributes {
        match attr {
            tl::enums::StarGiftAttribute::Model(attr) => {
                let id =
                    TelegramDocumentId::new(attr.document.id()).map_err(|_| Error::InvalidData)?;
                if model.replace(id).is_some() {
                    return Err(Error::InvalidData);
                }
            }
            tl::enums::StarGiftAttribute::Backdrop(attr) => {
                if backdrop.replace(attr.backdrop_id).is_some() {
                    return Err(Error::InvalidData);
                }
            }
            tl::enums::StarGiftAttribute::Pattern(attr) => {
                let id =
                    TelegramDocumentId::new(attr.document.id()).map_err(|_| Error::InvalidData)?;
                if pattern.replace(id).is_some() {
                    return Err(Error::InvalidData);
                }
            }
            _ => {}
        }
    }
    Ok(TelegramGiftDetails {
        id: gift
            .id
            .to_string()
            .try_into()
            .map_err(|_| Error::InvalidData)?,
        slug: resolved,
        collection: TelegramGiftId::new(gift.gift_id).map_err(|_| Error::InvalidData)?,
        number,
        model: model.ok_or(Error::InvalidData)?,
        backdrop,
        pattern,
    })
}
struct UserConnection {
    client: Client,
    handle: SenderPoolHandle,
    runner: tokio::task::JoinHandle<()>,
    stopped: CancellationToken,
}
impl Drop for UserConnection {
    fn drop(&mut self) {
        self.stopped.cancel();
        self.handle.quit();
        self.runner.abort();
    }
}
impl UserConnection {
    async fn open(account: &Account) -> Result<Self, BotError> {
        let session = Arc::new(MemorySession::default());
        session
            .set_home_dc_id(account.dc_id)
            .await
            .map_err(|_| BotError::Session)?;
        let mut dc = session
            .dc_option(account.dc_id)
            .map_err(|_| BotError::Session)?
            .ok_or(BotError::Session)?;
        dc.auth_key = Some(account.auth_key);
        match account.server_address {
            SocketAddr::V4(address) => dc.ipv4 = address,
            SocketAddr::V6(address) => dc.ipv6 = address,
        }
        session
            .set_dc_option(&dc)
            .await
            .map_err(|_| BotError::Session)?;
        let pool = SenderPool::with_configuration(
            session,
            account.api_id,
            ConnectionParams {
                use_ipv6: account.server_address.is_ipv6(),
                retry_policy: Box::new(NoRetries),
                updates_channel_capacity: NonZeroUsize::new(1).unwrap(),
                ..Default::default()
            },
        );
        let handle = pool.handle.thin.clone();
        let client = Client::new(pool.handle);
        let stopped = CancellationToken::new();
        let guard = stopped.clone().drop_guard();
        drop(pool.updates);
        let runner = tokio::spawn(async move {
            let _guard = guard;
            pool.runner.run().await;
        });
        Ok(Self {
            client,
            handle,
            runner,
            stopped,
        })
    }
    async fn close(&mut self) {
        self.stopped.cancel();
        self.handle.quit();
        if tokio::time::timeout(Duration::from_secs(5), &mut self.runner)
            .await
            .is_err()
        {
            self.runner.abort();
            let _ = (&mut self.runner).await;
        }
    }
}
async fn run_account(
    account: Account,
    directory: PathBuf,
    resolver: AccountGiftResolver,
    index: usize,
    scheduler: market_invocations::Handle,
    stop: CancellationToken,
) {
    if stop.is_cancelled() {
        return;
    }
    let fingerprint = Sha256::digest(account.auth_key);
    let bootstrap_owner = u64::from_be_bytes(fingerprint[..8].try_into().unwrap()) | (1 << 63);
    let path = directory.join(format!("account-{:x}.lock", fingerprint));
    let locked = tokio::task::spawn_blocking(move || -> Result<File, BotError> {
        std::fs::create_dir_all(directory).map_err(|_| BotError::Session)?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(path).map_err(|_| BotError::Session)?;
        lock.try_lock().map_err(|_| {
            BotError::Configuration("Another worker owns this Telegram user session")
        })?;
        Ok(lock)
    })
    .await;
    let Ok(Ok(_lock)) = locked else {
        tracing::warn!(
            account_index = index,
            "Telegram user session lock unavailable"
        );
        stop.cancelled().await;
        return;
    };
    loop {
        if stop.is_cancelled() {
            return;
        }
        let mut connection = match UserConnection::open(&account).await {
            Ok(connection) => connection,
            Err(_) => {
                tracing::warn!(account_index = index, "Telegram user session unavailable");
                stop.cancelled().await;
                return;
            }
        };
        let client = connection.client.clone();
        let executor = Executor {
            handle: scheduler.clone(),
            account: bootstrap_owner,
            stopped: connection.stopped.clone(),
        };
        let authenticated = tokio::select! {
            _ = stop.cancelled() => { connection.close().await; return; },
            user = executor.run(Group::Control, Priority::Interactive, Replay::Safe, move || { let client = client.clone(); async move { client.get_me().await } }) => user,
        };
        let mut fatal = false;
        match authenticated {
            Ok(user) if !user.is_bot() => {
                if let Some(id) = user.id().bare_id().and_then(|id| u64::try_from(id).ok()) {
                    let live = Arc::new(Live {
                        client: connection.client.clone(),
                        executor: Executor {
                            account: id,
                            ..executor
                        },
                    });
                    resolver.0.live.write().unwrap()[index] = Some(live.clone());
                    tracing::info!(
                        account_index = index,
                        "Telegram user gift lookup session authenticated"
                    );
                    tokio::select! { _ = stop.cancelled() => {}, _ = connection.stopped.cancelled() => {} }
                    let mut slots = resolver.0.live.write().unwrap();
                    if slots[index]
                        .as_ref()
                        .is_some_and(|value| Arc::ptr_eq(value, &live))
                    {
                        slots[index] = None;
                    }
                } else {
                    fatal = true;
                }
            }
            Ok(_) => {
                fatal = true;
                tracing::warn!(
                    account_index = index,
                    "Gift slug lookup requires a user session"
                );
            }
            Err(error) => {
                let error = error.bot();
                fatal = matches!(error, BotError::Authorization);
                tracing::warn!(account_index=index, %error, "Telegram user session authentication unavailable");
            }
        }
        connection.close().await;
        if fatal {
            stop.cancelled().await;
            return;
        }
        tokio::select! { _ = stop.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(5)) => {} }
    }
}

#[cfg(test)]
mod tests;
