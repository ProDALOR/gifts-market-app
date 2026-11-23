//! Interactive user authorization transport. Prompts belong to the CLI; exports share the importer schema.
use super::{
    accounts::{Export, validate_exports},
    invocations::{CallError, Executor, Group},
};
use crate::BotError;
use futures_util::FutureExt;
use grammers_client::{
    Client, SignInError,
    client::{LoginToken, PasswordToken},
    peer::User,
    sender::{ConnectionParams, InvocationError, NoRetries, SenderPool, SenderPoolHandle},
};
use grammers_session::{Session, storages::MemorySession};
use market_invocations::{Priority, Replay};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStep {
    CodeRequired,
    InvalidCode,
    PasswordRequired,
    InvalidPassword,
    Authorized {
        user_id: u64,
        username: Option<String>,
    },
}
pub struct UserLogin {
    api_id: i32,
    api_hash: String,
    session: Arc<MemorySession>,
    client: Client,
    handle: SenderPoolHandle,
    runner: Option<tokio::task::JoinHandle<()>>,
    executor: Executor,
    code: Option<Arc<LoginToken>>,
    password: Option<PasswordToken>,
    authorized: bool,
}
impl UserLogin {
    pub fn start(
        api_id: i32,
        api_hash: String,
        invocations: market_invocations::Handle,
    ) -> Result<Self, BotError> {
        if api_id <= 0 || api_hash.len() != 32 || !api_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(BotError::Configuration(
                "Telegram API ID must be positive and API hash must contain 32 hexadecimal characters",
            ));
        }
        let session = Arc::new(MemorySession::default());
        let pool = SenderPool::with_configuration(
            session.clone(),
            api_id,
            ConnectionParams {
                retry_policy: Box::new(NoRetries),
                updates_channel_capacity: NonZeroUsize::new(1).unwrap(),
                ..Default::default()
            },
        );
        let handle = pool.handle.thin.clone();
        let client = Client::new(pool.handle);
        drop(pool.updates);
        let stopped = CancellationToken::new();
        let guard = stopped.clone().drop_guard();
        let runner = tokio::spawn(async move {
            let _guard = guard;
            pool.runner.run().await;
        });
        Ok(Self {
            api_id,
            api_hash,
            session,
            client,
            handle,
            runner: Some(runner),
            executor: Executor {
                handle: invocations,
                account: api_id as u64,
                stopped,
            },
            code: None,
            password: None,
            authorized: false,
        })
    }
    pub async fn request_code(&mut self, phone: String) -> Result<(), BotError> {
        if self.code.is_some() || self.password.is_some() || self.authorized {
            return Err(BotError::Configuration("Login has already started"));
        }
        let digits = phone.strip_prefix('+').ok_or(BotError::Configuration(
            "Use a phone number with +country code",
        ))?;
        if !(5..=15).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(BotError::Configuration(
                "Use a phone number with +country code",
            ));
        }
        let client = self.client.clone();
        let hash = self.api_hash.clone();
        let result = self.executor.run(
            Group::Control,
            Priority::Interactive,
            Replay::Never,
            move || {
                let client = client.clone();
                let phone = phone.clone();
                let hash = hash.clone();
                async move {
                    // Grammers does not implement all login-payment flows; keep failures local.
                    let request = client.request_login_code(&phone, &hash);
                    match std::panic::AssertUnwindSafe(request).catch_unwind().await {
                        Ok(result) => result.map(Ok),
                        Err(_) => Ok(Err(BotError::Configuration(
                            "Telegram requested an unsupported login flow; use an official Telegram app",
                        ))),
                    }
                }
            },
        ).await.map_err(CallError::bot)??;
        self.code = Some(Arc::new(result));
        Ok(())
    }
    pub async fn submit_code(&mut self, code: String) -> Result<LoginStep, BotError> {
        let token = self
            .code
            .clone()
            .ok_or(BotError::Configuration("Request a login code first"))?;
        let client = self.client.clone();
        let result = self
            .executor
            .run(
                Group::Control,
                Priority::Interactive,
                Replay::Never,
                move || {
                    let client = client.clone();
                    let token = token.clone();
                    let code = code.clone();
                    async move { network_result(client.sign_in(&token, &code).await) }
                },
            )
            .await
            .map_err(CallError::bot)?;
        self.apply(result)
    }
    pub async fn submit_password(&mut self, password: String) -> Result<LoginStep, BotError> {
        let token = self.password.take().ok_or(BotError::Configuration(
            "No two-factor password challenge is pending",
        ))?;
        let token = Arc::new(Mutex::new(Some(token)));
        let client = self.client.clone();
        let result = self
            .executor
            .run(
                Group::Control,
                Priority::Interactive,
                Replay::Never,
                move || {
                    let token = token.lock().unwrap().take();
                    let client = client.clone();
                    let password = password.clone();
                    async move {
                        match token {
                            Some(token) => {
                                network_result(client.check_password(token, password).await)
                            }
                            None => Err(InvocationError::Dropped(
                                "password challenge already consumed",
                            )),
                        }
                    }
                },
            )
            .await
            .map_err(CallError::bot)?;
        self.apply(result)
    }
    fn apply(&mut self, result: Result<User, SignInError>) -> Result<LoginStep, BotError> {
        match result {
            Ok(user) => {
                let user_id = user.id().bare_id().ok_or(BotError::InvalidResponse)? as u64;
                let username = user.username().map(str::to_owned);
                self.authorized = true;
                self.code = None;
                self.password = None;
                Ok(LoginStep::Authorized { user_id, username })
            }
            Err(SignInError::InvalidCode) => Ok(LoginStep::InvalidCode),
            Err(SignInError::PasswordRequired(token)) => {
                self.code = None;
                self.password = Some(token);
                Ok(LoginStep::PasswordRequired)
            }
            Err(SignInError::InvalidPassword(token)) => {
                self.password = Some(token);
                Ok(LoginStep::InvalidPassword)
            }
            Err(SignInError::SignUpRequired) => Err(BotError::Configuration(
                "Create the Telegram account in an official app before logging in",
            )),
            Err(SignInError::Other(error)) => Err(super::client::rpc(error)),
        }
    }
    fn export(&self) -> Result<Export, BotError> {
        if !self.authorized {
            return Err(BotError::Configuration(
                "Complete user login before saving a session",
            ));
        }
        let dc_id = self.session.home_dc_id().map_err(|_| BotError::Session)?;
        let dc = self
            .session
            .dc_option(dc_id)
            .map_err(|_| BotError::Session)?
            .ok_or(BotError::Session)?;
        let key = dc.auth_key.ok_or(BotError::Session)?;
        let auth_key = key.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Export {
            api_id: self.api_id,
            auth_key,
            dc_id,
            server_address: dc.ipv4.into(),
        })
    }
    pub async fn close(mut self) {
        self.executor.stopped.cancel();
        self.handle.quit();
        if let Some(mut runner) = self.runner.take()
            && tokio::time::timeout(Duration::from_secs(5), &mut runner)
                .await
                .is_err()
        {
            runner.abort();
            let _ = runner.await;
        }
    }
}
impl Drop for UserLogin {
    fn drop(&mut self) {
        self.executor.stopped.cancel();
        self.handle.quit();
        if let Some(runner) = self.runner.take() {
            runner.abort();
        }
    }
}
fn network_result(
    result: Result<User, SignInError>,
) -> Result<Result<User, SignInError>, InvocationError> {
    match result {
        Err(SignInError::Other(error)) => Err(error),
        other => Ok(other),
    }
}

/// Exclusive, atomic append preserving existing records and their unknown fields.
/// Holds the destination lease before a login is attempted. No secrets are printed or exposed.
pub struct AccountExports {
    path: PathBuf,
    records: Vec<serde_json::Value>,
    temporary: tempfile::NamedTempFile,
    _lock: File,
}
impl AccountExports {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|_| anyhow::anyhow!("Cannot create accounts export directory"))?;
        let name = path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Choose an accounts export file"))?;
        let mut lock_name = name.to_os_string();
        lock_name.push(".lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(parent.join(lock_name))
            .map_err(|_| anyhow::anyhow!("Cannot lock accounts export"))?;
        lock.try_lock()
            .map_err(|_| anyhow::anyhow!("Another login is writing this accounts export"))?;
        let records = match File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(65537)
                    .read_to_end(&mut bytes)
                    .map_err(|_| anyhow::anyhow!("Cannot read accounts export"))?;
                if bytes.len() > 65536 {
                    anyhow::bail!("Accounts export exceeds 64 KiB");
                }
                let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes)
                    .map_err(|_| anyhow::anyhow!("Invalid accounts export JSON"))?;
                if !rows.is_empty() {
                    validate_exports(&bytes)?;
                }
                if rows.len() >= 16 {
                    anyhow::bail!("Accounts export already contains 16 sessions");
                }
                rows
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => anyhow::bail!("Cannot read accounts export"),
        };
        let temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| anyhow::anyhow!("Cannot prepare accounts export"))?;
        Ok(Self {
            path: path.to_owned(),
            records,
            temporary,
            _lock: lock,
        })
    }
    pub fn save(self, login: &UserLogin) -> anyhow::Result<()> {
        self.append(login.export()?)
    }
    fn append(mut self, export: Export) -> anyhow::Result<()> {
        self.records.push(
            serde_json::to_value(export)
                .map_err(|_| anyhow::anyhow!("Cannot encode user session"))?,
        );
        let bytes = serde_json::to_vec_pretty(&self.records)
            .map_err(|_| anyhow::anyhow!("Cannot encode accounts export"))?;
        validate_exports(&bytes)?;
        self.temporary
            .write_all(&bytes)
            .map_err(|_| anyhow::anyhow!("Cannot write accounts export"))?;
        self.temporary
            .as_file()
            .sync_all()
            .map_err(|_| anyhow::anyhow!("Cannot synchronize accounts export"))?;
        self.temporary
            .persist(&self.path)
            .map_err(|_| anyhow::anyhow!("Cannot replace accounts export"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
