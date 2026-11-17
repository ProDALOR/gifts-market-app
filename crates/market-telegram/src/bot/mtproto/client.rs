//! Grammers transport. Telegram credentials and raw MTProto types stay in this module.
use super::{
    cooldown::RateLimits,
    requests::{command_request, menu_request, reply_request},
    session_store::DurableSession,
};
use crate::bot::{
    BotCredentials, BotError, commands,
    transport::{BotUser, Delivery, Message, TelegramActions, Transport, UpdateCheckpoint},
};
use grammers_client::{
    Client,
    client::UpdateStream,
    peer::User,
    sender::{
        ConnectionParams, InvocationError, NoRetries, SenderPool, SenderPoolHandle,
        UpdatesConfiguration,
    },
    tl,
    update::Update,
};
use grammers_session::{
    Session,
    types::{PeerId, PeerKind},
    updates::State,
};
use std::{num::NonZeroUsize, sync::Arc};
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Clone)]
pub(in crate::bot) struct MtProtoActions {
    client: Client,
    session: Arc<DurableSession>,
    credentials: BotCredentials,
    exited: CancellationToken,
    limits: RateLimits,
    documents: super::media::Documents,
}
pub(in crate::bot) struct MtProtoTransport {
    actions: MtProtoActions,
    updates: Option<tokio::sync::mpsc::Receiver<grammers_session::updates::UpdatesLike>>,
    stream: Option<UpdateStream>,
    handle: SenderPoolHandle,
    runner: Option<tokio::task::JoinHandle<()>>,
}
impl MtProtoTransport {
    pub fn new(
        session: Arc<DurableSession>,
        credentials: BotCredentials,
        limits: RateLimits,
    ) -> Self {
        let parameters = ConnectionParams {
            retry_policy: Box::new(NoRetries),
            updates_channel_capacity: NonZeroUsize::new(512).unwrap(),
            ..ConnectionParams::default()
        };
        let pool = SenderPool::with_configuration(session.clone(), credentials.api_id, parameters);
        let handle = pool.handle.thin.clone();
        let client = Client::new(pool.handle);
        let exited = CancellationToken::new();
        let guard = exited.clone().drop_guard();
        let runner = tokio::spawn(async move {
            let _guard = guard;
            pool.runner.run().await;
        });
        Self {
            actions: MtProtoActions {
                client,
                session,
                credentials,
                exited,
                limits,
                documents: super::media::Documents::default(),
            },
            updates: Some(pool.updates),
            stream: None,
            handle,
            runner: Some(runner),
        }
    }
    pub fn stopped(&self) -> CancellationToken {
        self.actions.exited.clone()
    }
    pub async fn close(mut self) -> Result<(), BotError> {
        self.handle.quit();
        let runner = self.runner.as_mut().ok_or(BotError::Runner)?;
        match tokio::time::timeout(std::time::Duration::from_secs(5), &mut *runner).await {
            Ok(result) => result.map_err(|_| BotError::Runner),
            Err(_) => {
                runner.abort();
                let _ = runner.await;
                Ok(())
            }
        }
    }
}
impl MtProtoActions {
    fn failure(&self, error: InvocationError) -> BotError {
        if self.exited.is_cancelled() {
            BotError::Runner
        } else {
            rpc(error)
        }
    }
}
impl Drop for MtProtoTransport {
    fn drop(&mut self) {
        self.handle.quit();
        if let Some(runner) = self.runner.take() {
            runner.abort();
        }
    }
}
impl TelegramActions for MtProtoActions {
    type GiftSource = super::gifts::TelegramGifts;
    async fn media(
        &self,
        key: market_media::MediaKey,
    ) -> Result<market_media::MediaAsset, market_media::MediaError> {
        if self.exited.is_cancelled() {
            return Err(market_media::MediaError::Unavailable);
        }
        super::media::get(&self.client, &self.limits.media, &self.documents, key).await
    }
    fn gift_source(&self, message: &Message) -> Self::GiftSource {
        super::gifts::TelegramGifts {
            client: self.client.clone(),
            peer: message.peer,
            cooldown: self.limits.gifts.clone(),
        }
    }
    async fn identity(&self) -> Result<BotUser, BotError> {
        let user = if self
            .client
            .is_authorized()
            .await
            .map_err(|error| self.failure(error))?
        {
            self.client
                .get_me()
                .await
                .map_err(|error| self.failure(error))?
        } else {
            // Reauthorizing an existing session would replace its saved update
            // position with a new baseline. Require explicit recovery instead.
            if self.session.peer(PeerId::self_user()).await?.is_some() {
                return Err(BotError::Authorization);
            }
            self.client
                .bot_sign_in(&self.credentials.token, &self.credentials.api_hash)
                .await
                .map_err(|error| self.failure(error))?
        };
        identity(user, self.credentials.bot_id())
    }
    async fn configure(&self, url: Option<&Url>) -> Result<(), BotError> {
        let _ = self
            .client
            .invoke(&command_request())
            .await
            .map_err(|error| self.failure(error))?;
        if let Some(url) = url {
            let _ = self
                .client
                .invoke(&menu_request(url))
                .await
                .map_err(|error| self.failure(error))?;
        }
        Ok(())
    }
    async fn send(
        &self,
        message: &Message,
        reply: &commands::Reply,
        url: Option<&Url>,
    ) -> Result<(), BotError> {
        if self.exited.is_cancelled() {
            return Err(BotError::Runner);
        }
        if let Some(seconds) = self.limits.replies.remaining() {
            return Err(BotError::RetryAfter(seconds));
        }
        let request = reply_request(self.credentials.bot_id(), message, reply, url);
        let _ = self.client.invoke(&request).await.map_err(|error| {
            let error = self.failure(error);
            if let BotError::RetryAfter(seconds) = error {
                self.limits.replies.observe(seconds);
            }
            error
        })?;
        Ok(())
    }
}
impl Transport for MtProtoTransport {
    type Actions = MtProtoActions;
    fn actions(&self) -> Self::Actions {
        self.actions.clone()
    }
    async fn next(&mut self) -> Result<Delivery, BotError> {
        if self
            .runner
            .as_ref()
            .is_none_or(|runner| runner.is_finished())
        {
            return Err(BotError::Runner);
        }
        if self.stream.is_none() {
            self.stream = Some(
                self.actions
                    .client
                    .stream_updates(
                        self.updates.take().ok_or(BotError::Runner)?,
                        UpdatesConfiguration { catch_up: true },
                    )
                    .await
                    .map_err(|_| BotError::Session)?,
            );
        }
        let (raw, state, peers) = self
            .stream
            .as_mut()
            .unwrap()
            .next_raw()
            .await
            .map_err(|error| self.actions.failure(error))?;
        let update = Update::from_raw(&self.actions.client, raw, state.clone(), peers);
        let message = if let Update::NewMessage(message) = update {
            if message.outgoing() || message.peer_id().kind() != PeerKind::User {
                None
            } else {
                let peer = message
                    .peer_ref()
                    .await
                    .map_err(|_| BotError::Session)?
                    .ok_or(BotError::InvalidResponse)?;
                Some(Message {
                    peer,
                    id: message.id(),
                    outgoing: false,
                    text: message.text().to_owned(),
                })
            }
        } else {
            None
        };
        Ok(Delivery { state, message })
    }
}

impl UpdateCheckpoint for MtProtoActions {
    async fn acknowledge(&self, state: State) -> Result<(), BotError> {
        self.session.acknowledge(state).await
    }
}

fn identity(user: User, expected: u64) -> Result<BotUser, BotError> {
    let id = user
        .id()
        .bare_id()
        .and_then(|id| u64::try_from(id).ok())
        .ok_or(BotError::InvalidResponse)?;
    if !user.is_bot() || id != expected {
        return Err(BotError::Configuration(
            "The saved session belongs to a different account",
        ));
    }
    let has_main_web_app = matches!(&user.raw,tl::enums::User::User(raw) if raw.bot_has_main_app);
    let aliases = match &user.raw {
        tl::enums::User::User(raw) => active_usernames(
            raw.username.as_deref(),
            raw.usernames.as_deref().unwrap_or(&[]),
        ),
        _ => Vec::new(),
    };
    Ok(BotUser {
        id,
        username: aliases.first().cloned().ok_or(BotError::Configuration(
            "Authenticated bot has no active username",
        ))?,
        active_usernames: aliases,
        first_name: user.first_name().unwrap_or("").to_owned(),
        has_main_web_app,
    })
}
fn active_usernames(primary: Option<&str>, aliases: &[tl::enums::Username]) -> Vec<String> {
    let mut names = Vec::<String>::new();
    for name in primary
        .into_iter()
        .chain(aliases.iter().filter_map(|alias| {
            let tl::enums::Username::Username(alias) = alias;
            alias.active.then_some(alias.username.as_str())
        }))
    {
        if !name.is_empty() && !names.iter().any(|other| other.eq_ignore_ascii_case(name)) {
            names.push(name.to_owned());
        }
    }
    names
}
pub(super) fn rpc(error: InvocationError) -> BotError {
    match error {
        InvocationError::Session(_) => BotError::Session,

        InvocationError::Rpc(error) if error.code == 420 => {
            BotError::RetryAfter(error.value.unwrap_or(1).max(1) as u64)
        }
        InvocationError::Rpc(error)
            if error.code == 401
                || matches!(
                    error.name.as_str(),
                    "ACCESS_TOKEN_INVALID"
                        | "ACCESS_TOKEN_EXPIRED"
                        | "API_ID_INVALID"
                        | "API_ID_PUBLISHED_FLOOD"
                        | "AUTH_KEY_DUPLICATED"
                        | "SESSION_REVOKED"
                        | "SESSION_EXPIRED"
                ) =>
        {
            BotError::Authorization
        }
        InvocationError::Rpc(error)
            if matches!(
                error.name.as_str(),
                "USER_IS_BLOCKED"
                    | "YOU_BLOCKED_USER"
                    | "INPUT_USER_DEACTIVATED"
                    | "USER_DEACTIVATED"
                    | "USER_DEACTIVATED_BAN"
                    | "PEER_ID_INVALID"
                    | "CHAT_WRITE_FORBIDDEN"
                    | "CHAT_SEND_PLAIN_FORBIDDEN"
                    | "CHAT_RESTRICTED"
            ) =>
        {
            BotError::PeerUnavailable
        }
        InvocationError::Rpc(error) => BotError::Rpc(error.code),
        _ => BotError::Transport,
    }
}

#[cfg(test)]
mod tests {
    use super::super::session_store::SessionStore;
    use super::*;
    use crate::bot::tests::credentials;
    use tokio_util::task::TaskTracker;

    #[test]
    fn collectible_bot_aliases_work_without_a_primary_username_and_inactive_names_are_excluded() {
        let alias = |name: &str, active| {
            tl::types::Username {
                editable: false,
                active,
                deletable: false,
                expired: false,
                username: name.into(),
            }
            .into()
        };
        let aliases = [
            alias("inactive_bot", false),
            alias("active_bot", true),
            alias("ACTIVE_BOT", true),
            alias("second_bot", true),
        ];
        let names = active_usernames(None, &aliases);
        assert_eq!(names, ["active_bot", "second_bot"]);
        assert_eq!(
            active_usernames(Some("primary_bot"), &aliases),
            ["primary_bot", "active_bot", "second_bot"]
        );
        let incoming = |text| commands::Incoming {
            private: true,
            user_id: Some(7),
            message_id: 1,
            text: Some(text),
        };
        assert!(commands::parse(incoming("/status@SECOND_BOT"), &names).is_some());
        assert!(commands::parse(incoming("/status@inactive_bot"), &names).is_none());
    }

    #[tokio::test]
    async fn sender_stops_without_opening_a_network_connection_and_reports_task_failure() {
        let directory = tempfile::tempdir().unwrap();
        let jobs = TaskTracker::new();
        let storage = SessionStore::open(directory.path(), 42, 12345, &jobs)
            .await
            .unwrap();
        MtProtoTransport::new(
            storage.session.clone(),
            credentials(),
            RateLimits::default(),
        )
        .close()
        .await
        .unwrap();
        let mut transport = MtProtoTransport::new(
            storage.session.clone(),
            credentials(),
            RateLimits::default(),
        );
        let exited = transport.stopped();
        let runner = transport.runner.take().unwrap();
        runner.abort();
        let _ = runner.await;
        assert!(
            exited.is_cancelled(),
            "supervision must wake even while a reply is in a flood wait"
        );
        transport.runner = Some(tokio::spawn(async { panic!("simulated sender failure") }));
        assert_eq!(transport.close().await, Err(BotError::Runner));
        jobs.close();
        jobs.wait().await;
        drop(storage);
        assert!(
            SessionStore::open(directory.path(), 42, 12345, &jobs)
                .await
                .is_ok()
        );
    }
}
