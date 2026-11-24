//! Grammers transport. Telegram credentials and raw MTProto types stay in this module.
use super::{
    invocations::{CallError, Executor, Group},
    requests::{command_request, menu_request},
    session_store::DurableSession,
};
use crate::interactions::{CallbackTarget, Event, InlineMessageId};
use crate::{
    BotCredentials, BotError, LaunchButton, MenuCommand, Reply,
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
use market_invocations::{Priority, Replay};
use std::{num::NonZeroUsize, sync::Arc};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct MtProtoActions {
    pub(super) client: Client,
    session: Arc<DurableSession>,
    pub(super) credentials: BotCredentials,
    exited: CancellationToken,
    pub(super) invocations: Executor,
    pub(super) profile: Arc<std::sync::OnceLock<BotUser>>,
    pub(super) artwork: Option<market_media::MediaCache>,
    pub(super) photos: super::photos::Photos,
    documents: super::media::Documents,
    inventory: Option<(super::gifts::InventoryClient, Arc<super::gifts::Live>)>,
}
pub struct MtProtoTransport {
    actions: MtProtoActions,
    updates: Option<tokio::sync::mpsc::Receiver<grammers_session::updates::UpdatesLike>>,
    stream: Option<UpdateStream>,
    handle: SenderPoolHandle,
    runner: Option<tokio::task::JoinHandle<()>>,
}
impl MtProtoTransport {
    #[cfg(test)]
    pub(super) fn take_test_updates(
        &mut self,
    ) -> tokio::sync::mpsc::Receiver<grammers_session::updates::UpdatesLike> {
        // Live probes match their own queries without catch-up or checkpoint acknowledgement.
        self.updates
            .take()
            .expect("unused live-test update receiver")
    }
    pub(crate) fn new(
        session: Arc<DurableSession>,
        credentials: BotCredentials,
        invocations: market_invocations::Handle,
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
                invocations: Executor {
                    handle: invocations,
                    account: credentials.bot_id(),
                    stopped: exited.clone(),
                    authorization: Default::default(),
                },
                credentials,
                exited,
                profile: Default::default(),
                artwork: None,
                photos: Default::default(),
                documents: super::media::Documents::default(),
                inventory: None,
            },
            updates: Some(pool.updates),
            stream: None,
            handle,
            runner: Some(runner),
        }
    }
    pub fn bind_artwork(&mut self, cache: market_media::MediaCache) {
        self.actions.artwork = Some(cache);
    }
    pub fn bind_inventory(
        &mut self,
        inventory: super::gifts::InventoryClient,
    ) -> Result<(), BotError> {
        inventory.bind(self.actions.credentials.bot_id())?;
        let live = Arc::new(super::gifts::Live {
            client: self.actions.client.clone(),
            session: self.actions.session.clone(),
            executor: self.actions.invocations.clone(),
        });
        self.actions.inventory = Some((inventory, live));
        Ok(())
    }
    pub fn stopped(&self) -> CancellationToken {
        self.actions.exited.clone()
    }
    pub async fn close(mut self) -> Result<(), BotError> {
        self.actions.exited.cancel();
        if let Some((client, live)) = &self.actions.inventory {
            client.clear(live);
        }
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
        self.actions.exited.cancel();
        if let Some((client, live)) = &self.actions.inventory {
            client.clear(live);
        }
        self.handle.quit();
        if let Some(runner) = self.runner.take() {
            runner.abort();
        }
    }
}
impl TelegramActions for MtProtoActions {
    async fn media(
        &self,
        key: market_media::MediaKey,
    ) -> Result<market_media::MediaAsset, market_media::MediaError> {
        if self.exited.is_cancelled() {
            return Err(market_media::MediaError::Unavailable);
        }
        super::media::get(&self.client, &self.invocations, &self.documents, key).await
    }
    async fn identity(&self) -> Result<BotUser, BotError> {
        let client = self.client.clone();
        let authorized = self
            .invocations
            .run(
                Group::Control,
                Priority::Interactive,
                Replay::Safe,
                move || {
                    let client = client.clone();
                    async move { client.is_authorized().await }
                },
            )
            .await
            .map_err(CallError::bot)?;
        let user = if authorized {
            let client = self.client.clone();
            self.invocations
                .run(
                    Group::Control,
                    Priority::Interactive,
                    Replay::Safe,
                    move || {
                        let client = client.clone();
                        async move { client.get_me().await }
                    },
                )
                .await
                .map_err(CallError::bot)?
        } else {
            // Reauthorizing an existing session would replace its saved update
            // position with a new baseline. Require explicit recovery instead.
            if self.session.peer(PeerId::self_user()).await?.is_some() {
                return Err(BotError::Authorization);
            }
            let client = self.client.clone();
            let credentials = self.credentials.clone();
            self.invocations
                .run(
                    Group::Control,
                    Priority::Interactive,
                    Replay::Never,
                    move || {
                        let client = client.clone();
                        let credentials = credentials.clone();
                        async move {
                            client
                                .bot_sign_in(&credentials.token, &credentials.api_hash)
                                .await
                        }
                    },
                )
                .await
                .map_err(CallError::bot)?
        };
        let identity = identity(user, self.credentials.bot_id())?;
        if let Some((client, live)) = &self.inventory {
            client.publish(live.clone());
        }
        let _ = self.profile.set(identity.clone());
        Ok(identity)
    }
    async fn configure(
        &self,
        commands: &[MenuCommand],
        launch: Option<&LaunchButton>,
    ) -> Result<(), BotError> {
        let _ = self
            .invocations
            .rpc(
                &self.client,
                command_request(commands),
                Group::Control,
                Priority::Interactive,
                Replay::Safe,
            )
            .await?;
        if let Some(launch) = launch {
            let _ = self
                .invocations
                .rpc(
                    &self.client,
                    menu_request(launch),
                    Group::Control,
                    Priority::Interactive,
                    Replay::Safe,
                )
                .await?;
        }
        Ok(())
    }
    async fn answer_callback(&self, event: &Event, handled: bool) -> Result<(), BotError> {
        let Event::Callback { id, .. } = event else {
            return Ok(());
        };
        match self
            .invocations
            .rpc(
                &self.client,
                tl::functions::messages::SetBotCallbackAnswer {
                    alert: false,
                    query_id: *id,
                    message: (!handled)
                        .then(|| "This button has expired. Request a fresh valuation.".into()),
                    url: None,
                    cache_time: 0,
                },
                Group::Inline,
                Priority::Interactive,
                Replay::Safe,
            )
            .await
        {
            Ok(_) | Err(BotError::Rpc(400)) => Ok(()),
            Err(error) => Err(error),
        }
    }
    async fn send_progress(
        &self,
        event: &Event,
        reply: &Reply,
        launch: Option<&LaunchButton>,
    ) -> Result<Option<Event>, BotError> {
        self.progress_reply(event, reply, launch).await
    }
    async fn send(
        &self,
        event: &Event,
        reply: Option<&Reply>,
        launch: Option<&LaunchButton>,
    ) -> Result<(), BotError> {
        if self.exited.is_cancelled() {
            return Err(BotError::Runner);
        }
        self.respond(event, reply, launch).await
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
        let event = match raw {
            tl::enums::Update::BotInlineQuery(query) if query.user_id > 0 => Some(Event::Inline {
                id: query.query_id,
                user_id: query.user_id as u64,
                query: query.query.chars().take(2048).collect(),
                offset: query.offset,
            }),
            tl::enums::Update::BotGuestChatQuery(query) => guest_event(query),
            tl::enums::Update::InlineBotCallbackQuery(query) if query.user_id > 0 => query
                .data
                .filter(|data| data.len() <= 64)
                .map(|data| Event::Callback {
                    id: query.query_id,
                    user_id: query.user_id as u64,
                    data,
                    target: CallbackTarget::Inline(InlineMessageId::from(&query.msg_id)),
                }),
            tl::enums::Update::BotCallbackQuery(query) if query.user_id > 0 => {
                if let Some(data) = query.data.filter(|data| data.len() <= 64) {
                    peers
                        .get_ref(PeerId::from(query.peer))
                        .await
                        .map_err(|_| BotError::Session)?
                        .map(|peer| Event::Callback {
                            id: query.query_id,
                            user_id: query.user_id as u64,
                            data,
                            target: CallbackTarget::Message(Message {
                                peer,
                                id: query.msg_id,
                                outgoing: true,
                                text: String::new(),
                            }),
                        })
                } else {
                    None
                }
            }
            raw => {
                let update = Update::from_raw(&self.actions.client, raw, state.clone(), peers);
                if let Update::NewMessage(message) = update {
                    if message.outgoing() || message.peer_id().kind() != PeerKind::User {
                        None
                    } else {
                        let peer = message
                            .peer_ref()
                            .await
                            .map_err(|_| BotError::Session)?
                            .ok_or(BotError::InvalidResponse)?;
                        Some(Event::Message(Message {
                            peer,
                            id: message.id(),
                            outgoing: false,
                            text: match &message.raw {
                                tl::enums::Update::NewMessage(raw) => reference_text(&raw.message),
                                _ => None,
                            }
                            .unwrap_or_else(|| message.text().chars().take(4096).collect()),
                        }))
                    }
                } else {
                    None
                }
            }
        };
        Ok(Delivery { state, event })
    }
}

fn reference_text(message: &tl::enums::Message) -> Option<String> {
    let mut text = match message {
        tl::enums::Message::Message(message) => {
            let mut text: String = message.message.chars().take(2048).collect();
            for entity in message
                .entities
                .as_deref()
                .unwrap_or_default()
                .iter()
                .take(16)
            {
                if let tl::enums::MessageEntity::TextUrl(entity) = entity
                    && market_gifts::GiftSlug::new(&entity.url).is_ok()
                {
                    text.push(' ');
                    text.push_str(&entity.url);
                }
            }
            text
        }
        tl::enums::Message::Service(message) => match &message.action {
            tl::enums::MessageAction::StarGiftUnique(action) => match &action.gift {
                tl::enums::StarGift::Unique(gift) => gift.slug.clone(),
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };
    text.truncate(text.floor_char_boundary(4096));
    Some(text)
}
pub(super) fn guest_event(query: tl::types::UpdateBotGuestChatQuery) -> Option<Event> {
    let tl::enums::Message::Message(message) = query.message else {
        return None;
    };
    let Some(tl::enums::Peer::User(sender)) = &message.from_id else {
        return None;
    };
    if sender.user_id <= 0 || message.out {
        return None;
    }
    Some(Event::Guest {
        id: query.query_id,
        user_id: sender.user_id as u64,
        text: reference_text(&tl::enums::Message::Message(message.clone())).unwrap_or_default(),
        references: query
            .reference_messages
            .unwrap_or_default()
            .into_iter()
            .take(4)
            .filter_map(|message| reference_text(&message))
            .collect(),
    })
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
        inline_enabled: matches!(&user.raw,tl::enums::User::User(raw) if raw.bot_inline_placeholder.is_some()),
        guest_enabled: matches!(&user.raw,tl::enums::User::User(raw) if raw.bot_guestchat),
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
pub fn rpc(error: InvocationError) -> BotError {
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
    use crate::tests::credentials;
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
    }

    #[tokio::test]
    async fn sender_stops_without_opening_a_network_connection_and_reports_task_failure() {
        let directory = tempfile::tempdir().unwrap();
        let jobs = TaskTracker::new();
        let invocations = market_invocations::Runtime::start(Default::default()).unwrap();
        let storage = SessionStore::open(directory.path(), 42, 12345, &jobs)
            .await
            .unwrap();
        MtProtoTransport::new(storage.session.clone(), credentials(), invocations.handle())
            .close()
            .await
            .unwrap();
        let mut transport =
            MtProtoTransport::new(storage.session.clone(), credentials(), invocations.handle());
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
        SessionStore::open(directory.path(), 42, 12345, &jobs)
            .await
            .expect("sender shutdown releases its session");
    }
}
