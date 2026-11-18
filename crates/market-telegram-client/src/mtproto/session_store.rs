//! Private MTProto session storage and an exclusive worker lock.
//! SQLite operations run on tracked blocking workers, outside the API event loop.
use crate::BotError;
use grammers_session::{
    BoxFuture, Session,
    storages::{SqliteSession, SqliteSessionError},
    types::{ChannelState, DcOption, PeerId, PeerInfo, UpdateState, UpdatesState},
    updates::{MessageBox, State},
};
use std::{
    fs::{File, OpenOptions},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio_util::task::TaskTracker;

type SqlFuture<T> = Pin<Box<dyn Future<Output = Result<T, SqliteSessionError>> + Send>>;
pub(crate) struct SessionStore {
    pub session: Arc<DurableSession>,
    pub path: PathBuf,
    _lock: File,
}
pub(crate) struct DurableSession {
    database: Arc<SqliteSession>,
    jobs: TaskTracker,
}
impl SessionStore {
    pub async fn open(
        directory: &Path,
        bot_id: u64,
        api_id: i32,
        jobs: &TaskTracker,
    ) -> Result<Self, BotError> {
        let directory = directory.to_owned();
        let (database, path, lock) = jobs
            .spawn_blocking(move || {
                std::fs::create_dir_all(&directory).map_err(|_| BotError::Session)?;
                let mut options = OpenOptions::new();
                options.read(true).write(true).create(true).truncate(false);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                // Keep the stable bot-scoped lock across API IDs and session upgrades.
                let lock = options
                    .open(directory.join(format!("{bot_id}.lock")))
                    .map_err(|_| BotError::Session)?;
                lock.try_lock().map_err(|_| {
                    BotError::Configuration("Another bot worker owns this session directory")
                })?;
                let path = directory.join(format!("{bot_id}-{api_id}.mtproto.sqlite"));
                let private = options.open(&path).map_err(|_| BotError::Session)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    private
                        .set_permissions(std::fs::Permissions::from_mode(0o600))
                        .map_err(|_| BotError::Session)?;
                }
                drop(private);
                let database = tokio::runtime::Handle::current()
                    .block_on(SqliteSession::open(&path))
                    .map_err(|_| BotError::Session)?;
                Ok::<_, BotError>((database, path, lock))
            })
            .await
            .map_err(|_| BotError::Session)??;
        Ok(Self {
            session: Arc::new(DurableSession {
                database: Arc::new(database),
                jobs: jobs.clone(),
            }),
            path,
            _lock: lock,
        })
    }
}
impl DurableSession {
    fn execute<T: Send + 'static>(
        &self,
        operation: impl FnOnce(Arc<SqliteSession>) -> SqlFuture<T> + Send + 'static,
    ) -> BoxFuture<'static, Result<T, BotError>> {
        let database = self.database.clone();
        let jobs = self.jobs.clone();
        Box::pin(async move {
            jobs.spawn_blocking(move || {
                tokio::runtime::Handle::current().block_on(operation(database))
            })
            .await
            .map_err(|_| BotError::Session)?
            .map_err(|_| BotError::Session)
        })
    }
    pub async fn acknowledge(&self, state: State) -> Result<(), BotError> {
        self.execute(move |database| {
            Box::pin(async move {
                let current = database.updates_state().await?;
                database
                    .set_update_state(UpdateState::All(advance(current, &state)))
                    .await
            })
        })
        .await
    }
}
fn advance(mut current: UpdatesState, state: &State) -> UpdatesState {
    current.date = current.date.max(state.date);
    current.seq = current.seq.max(state.seq);
    match state.message_box {
        Some(MessageBox::Common { pts }) => current.pts = current.pts.max(pts),
        Some(MessageBox::Secondary { qts }) => current.qts = current.qts.max(qts),
        Some(MessageBox::Channel { channel_id, pts }) => {
            if let Some(channel) = current
                .channels
                .iter_mut()
                .find(|channel| channel.id == channel_id)
            {
                channel.pts = channel.pts.max(pts);
            } else {
                current.channels.push(ChannelState {
                    id: channel_id,
                    pts,
                });
            }
        }
        None => {}
    }
    current
}
impl Session for DurableSession {
    type Error = BotError;
    fn home_dc_id(&self) -> Result<i32, BotError> {
        self.database.home_dc_id().map_err(|_| BotError::Session)
    }
    fn dc_option(&self, id: i32) -> Result<Option<DcOption>, BotError> {
        self.database.dc_option(id).map_err(|_| BotError::Session)
    }
    fn set_home_dc_id(&self, id: i32) -> BoxFuture<'_, Result<(), BotError>> {
        self.execute(move |db| Box::pin(async move { db.set_home_dc_id(id).await }))
    }
    fn set_dc_option(&self, option: &DcOption) -> BoxFuture<'_, Result<(), BotError>> {
        let option = option.clone();
        self.execute(move |db| Box::pin(async move { db.set_dc_option(&option).await }))
    }
    fn peer(&self, id: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, BotError>> {
        self.execute(move |db| Box::pin(async move { db.peer(id).await }))
    }
    fn cache_peer(&self, peer: PeerInfo) -> BoxFuture<'_, Result<(), BotError>> {
        self.execute(move |db| Box::pin(async move { db.cache_peer(peer).await }))
    }
    fn cache_peers(&self, peers: Vec<PeerInfo>) -> BoxFuture<'_, Result<(), BotError>> {
        self.execute(move |db| Box::pin(async move { db.cache_peers(peers).await }))
    }
    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, BotError>> {
        self.execute(move |db| Box::pin(async move { db.updates_state().await }))
    }
    fn set_update_state(&self, state: UpdateState) -> BoxFuture<'_, Result<(), BotError>> {
        self.execute(move |db| Box::pin(async move { db.set_update_state(state).await }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grammers_session::types::PeerAuth;

    #[test]
    fn child_process_cannot_take_the_worker_lock() {
        let Some(path) = std::env::var_os("MARKET_TEST_SESSION_LOCK") else {
            return;
        };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        assert!(file.try_lock().is_err());
    }
    #[tokio::test]
    async fn worker_lock_is_exclusive_across_processes_and_released_after_close() {
        let directory = tempfile::tempdir().unwrap();
        let jobs = TaskTracker::new();
        let store = SessionStore::open(directory.path(), 42, 12345, &jobs)
            .await
            .unwrap();
        let path = directory.path().join("42.lock");
        let child_path = path.clone();
        let output = tokio::task::spawn_blocking(move || {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "bot::mtproto::session_store::tests::child_process_cannot_take_the_worker_lock",
                ])
                .env("MARKET_TEST_SESSION_LOCK", child_path)
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        drop(store);
        jobs.close();
        jobs.wait().await;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        file.try_lock().unwrap();
    }
    #[tokio::test]
    async fn session_persists_login_peers_and_only_acknowledged_progress_under_exclusive_lock() {
        let directory = tempfile::tempdir().unwrap();
        let old_offset = directory.path().join("42.offset");
        std::fs::write(&old_offset, 13u64.to_be_bytes()).unwrap();
        let jobs = TaskTracker::new();
        let storage = SessionStore::open(directory.path(), 42, 12345, &jobs)
            .await
            .unwrap();
        assert!(
            SessionStore::open(directory.path(), 42, 12345, &jobs)
                .await
                .is_err()
        );
        assert!(
            SessionStore::open(directory.path(), 42, 67890, &jobs)
                .await
                .is_err()
        );
        let mut dc = storage.session.dc_option(2).unwrap().unwrap();
        dc.auth_key = Some([7; 256]);
        storage.session.set_dc_option(&dc).await.unwrap();
        storage.session.set_home_dc_id(2).await.unwrap();
        let peer = PeerInfo::User {
            id: 7,
            auth: Some(PeerAuth::from_hash(123)),
            bot: Some(true),
            is_self: Some(true),
        };
        storage.session.cache_peer(peer.clone()).await.unwrap();
        let initial = UpdatesState {
            pts: 10,
            qts: 20,
            date: 30,
            seq: 40,
            channels: vec![ChannelState { id: 100, pts: 50 }],
        };
        storage
            .session
            .set_update_state(UpdateState::All(initial.clone()))
            .await
            .unwrap();
        // A buffered later event is intentionally not acknowledged.
        storage
            .session
            .acknowledge(State {
                date: 31,
                seq: 41,
                message_box: Some(MessageBox::Common { pts: 11 }),
            })
            .await
            .unwrap();
        let _buffered = State {
            date: 32,
            seq: 42,
            message_box: Some(MessageBox::Common { pts: 12 }),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&storage.path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        drop(storage);
        jobs.close();
        jobs.wait().await;
        let restored = SessionStore::open(directory.path(), 42, 12345, &jobs)
            .await
            .unwrap();
        assert_eq!(restored.session.home_dc_id().unwrap(), 2);
        assert_eq!(
            restored.session.dc_option(2).unwrap().unwrap().auth_key,
            Some([7; 256])
        );
        assert_eq!(
            restored
                .session
                .peer(PeerId::user(7).unwrap())
                .await
                .unwrap(),
            Some(peer)
        );
        let expected = UpdatesState {
            pts: 11,
            date: 31,
            seq: 41,
            ..initial
        };
        assert_eq!(restored.session.updates_state().await.unwrap(), expected);
        assert_eq!(std::fs::read(old_offset).unwrap(), 13u64.to_be_bytes());
    }
    #[test]
    fn progress_merges_independent_message_boxes_without_regression() {
        let initial = UpdatesState {
            pts: 10,
            qts: 20,
            date: 30,
            seq: 40,
            channels: vec![ChannelState { id: 100, pts: 50 }],
        };
        let mut state = initial.clone();
        for message_box in [
            Some(MessageBox::Common { pts: 9 }),
            Some(MessageBox::Secondary { qts: 19 }),
            Some(MessageBox::Channel {
                channel_id: 100,
                pts: 49,
            }),
            None,
        ] {
            state = advance(
                state,
                &State {
                    date: 29,
                    seq: 39,
                    message_box,
                },
            );
            assert_eq!(state, initial);
        }
        state = advance(
            state,
            &State {
                date: 31,
                seq: 41,
                message_box: Some(MessageBox::Secondary { qts: 21 }),
            },
        );
        state = advance(
            state,
            &State {
                date: 31,
                seq: 41,
                message_box: Some(MessageBox::Channel {
                    channel_id: 101,
                    pts: 1,
                }),
            },
        );
        assert_eq!((state.pts, state.qts, state.channels.len()), (10, 21, 2));
        assert_eq!(state.channels[0].pts, 50);
    }
}
