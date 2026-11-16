//! One owner for the worker lock, sender runner and tracked session writes.
use super::{client::MtProtoTransport, cooldown::RateLimits, session_store::SessionStore};
use crate::bot::{BotCredentials, BotError};
use std::path::{Path, PathBuf};
use tokio_util::task::TaskTracker;
pub(in crate::bot) struct Connection {
    pub transport: MtProtoTransport,
    store: SessionStore,
    jobs: TaskTracker,
}
impl Connection {
    pub async fn open(
        credentials: BotCredentials,
        directory: &Path,
        limits: RateLimits,
    ) -> Result<Self, BotError> {
        let jobs = TaskTracker::new();
        let store =
            SessionStore::open(directory, credentials.bot_id(), credentials.api_id, &jobs).await?;
        let transport = MtProtoTransport::new(store.session.clone(), credentials, limits);
        Ok(Self {
            transport,
            store,
            jobs,
        })
    }
    pub fn session_path(&self) -> PathBuf {
        self.store.path.clone()
    }
    pub async fn close(self) -> Result<(), BotError> {
        let Self {
            transport,
            store,
            jobs,
        } = self;
        let result = transport.close().await;
        jobs.close();
        jobs.wait().await;
        drop(store);
        result
    }
}
