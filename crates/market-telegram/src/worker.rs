//! Bot lifecycle and startup. Command scheduling lives in the dispatcher.
use super::{
    BotConfig, BotCredentials, BotError, BotUser, LaunchButton, MessageHandler,
    dispatcher::{self, Limits, retry},
    transport::{TelegramActions, Transport},
};
use futures_util::FutureExt;
use market_invocations::{Handle as InvocationHandle, Runtime as InvocationRuntime};
use market_telegram_client::{Connection, app_url};
use std::{panic::AssertUnwindSafe, path::PathBuf};
use tokio_util::sync::CancellationToken;
use url::Url;
type IdentityObserver = Box<dyn Fn(&BotUser) + Send + Sync>;

pub struct TelegramWorker {
    credentials: BotCredentials,
    url: Option<Url>,
    data_dir: PathBuf,
    identity_observer: Option<IdentityObserver>,
    media: Option<tokio::sync::Mutex<market_media::MediaDownloader>>,
}
pub struct BotInspection {
    pub user: BotUser,
    pub session_path: PathBuf,
}
pub async fn inspect(
    credentials: BotCredentials,
    data_dir: PathBuf,
) -> anyhow::Result<BotInspection> {
    let invocations = InvocationRuntime::start(Default::default())?;
    let connection = Connection::open(credentials, &data_dir, invocations.handle()).await?;
    let session_path = connection.session_path();
    let result = AssertUnwindSafe(connection.transport.actions().identity())
        .catch_unwind()
        .await;
    let closed = connection.close().await;
    invocations.shutdown().await;
    let user = match result {
        Ok(result) => result?,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    closed?;
    Ok(BotInspection { user, session_path })
}
impl TelegramWorker {
    pub fn new(config: BotConfig) -> anyhow::Result<Self> {
        Ok(Self {
            credentials: config.credentials,
            url: config
                .public_url
                .filter(|url| !url.is_empty())
                .map(|url| app_url(&url))
                .transpose()?,
            data_dir: config.data_dir,
            identity_observer: None,
            media: None,
        })
    }
    /// Observe the authenticated profile after each login or session reconnect.
    /// Runs before command configuration; observers must be quick and nonblocking.
    pub fn on_authenticated(&mut self, observer: impl Fn(&BotUser) + Send + Sync + 'static) {
        self.identity_observer = Some(Box::new(observer));
    }
    /// Attach background media work to the same authenticated transport generation.
    pub fn set_media_downloader(&mut self, downloader: market_media::MediaDownloader) {
        self.media = Some(tokio::sync::Mutex::new(downloader));
    }
    pub async fn configure<H: MessageHandler>(&self, handler: &H) -> Result<BotUser, BotError> {
        let invocations =
            InvocationRuntime::start(Default::default()).map_err(|_| BotError::Runner)?;
        let result = self
            .execute(
                CancellationToken::new(),
                false,
                handler,
                invocations.handle(),
            )
            .await;
        invocations.shutdown().await;
        result?.ok_or(BotError::Runner)
    }
    pub async fn run<H: MessageHandler>(
        self,
        handler: H,
        stop: CancellationToken,
    ) -> Result<(), BotError> {
        let invocations =
            InvocationRuntime::start(Default::default()).map_err(|_| BotError::Runner)?;
        let result = self
            .run_with_invocations(&handler, stop, invocations.handle())
            .await;
        invocations.shutdown().await;
        result
    }
    /// Use an application-owned scheduler, shared across consumers and reconnects.
    pub async fn run_with_invocations<H: MessageHandler>(
        &self,
        handler: &H,
        stop: CancellationToken,
        invocations: InvocationHandle,
    ) -> Result<(), BotError> {
        let mut backoff = 1;
        loop {
            match self
                .execute(stop.clone(), true, handler, invocations.clone())
                .await
            {
                Err(BotError::Runner) => {
                    // Reopen the saved session only after all old session work has drained.
                    tokio::select! {_=stop.cancelled()=>return Ok(()), result=retry(BotError::Transport,&mut backoff)=>result?}
                }
                result => return result.map(|_| ()),
            }
        }
    }
    async fn execute<H: MessageHandler>(
        &self,
        stop: CancellationToken,
        consume: bool,
        handler: &H,
        invocations: InvocationHandle,
    ) -> Result<Option<BotUser>, BotError> {
        if stop.is_cancelled() {
            return Ok(None);
        }
        if invocations.is_closed() {
            return Err(BotError::Invocations);
        }
        let scheduler = invocations.clone();
        // Finish opening storage before cancellation may release the worker lock.
        let mut connection =
            Connection::open(self.credentials.clone(), &self.data_dir, invocations).await?;
        let exited = connection.transport.stopped();
        let result = AssertUnwindSafe(async {
            tokio::select! {
                biased;
                _=stop.cancelled()=>Ok(None),
                _=exited.cancelled()=>Err(BotError::Runner),
                _=scheduler.closed()=>Err(BotError::Invocations),
                result=self.serve(&mut connection.transport,consume,handler)=>result.map(Some),
            }
        })
        .catch_unwind()
        .await;
        let closed = connection.close().await;
        match result {
            Ok(result) => {
                let result = result?;
                closed?;
                Ok(result)
            }
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
    async fn serve<T: Transport, H: MessageHandler>(
        &self,
        transport: &mut T,
        consume: bool,
        handler: &H,
    ) -> Result<BotUser, BotError> {
        let actions = transport.actions();
        let mut backoff = 1;
        let user = loop {
            match actions.identity().await {
                Ok(user) => break user,
                Err(error) => retry(error, &mut backoff).await?,
            }
        };
        if let Some(observer) = &self.identity_observer {
            observer(&user);
        }
        let menu = handler.commands();
        let launch = self.url.as_ref().map(|url| LaunchButton {
            text: handler.launch_label().into(),
            url: url.clone(),
        });
        loop {
            match actions.configure(&menu, launch.as_ref()).await {
                Ok(()) => break,
                Err(error) => retry(error, &mut backoff).await?,
            }
        }
        if !consume {
            return Ok(user);
        }
        tracing::info!(bot_id = user.id, bot_username = %user.username, "MTProto bot worker started");
        let commands = dispatcher::run(
            transport,
            handler,
            launch.as_ref(),
            &user.active_usernames,
            Limits::default(),
        );
        let media = async {
            if let Some(media) = &self.media {
                let mut downloader = media.lock().await;
                downloader.run(|id| actions.media(id)).await;
            }
            std::future::pending::<()>().await;
        };
        tokio::select! {
            result = commands => result?,
            _ = media => unreachable!(),
        }
        Ok(user)
    }
}
#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
