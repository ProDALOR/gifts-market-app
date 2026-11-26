//! Native bot actions and update transport contracts, without TL request types.
use super::{BotError, IncomingMessage, LaunchButton, MenuCommand, Reply};
use grammers_session::{
    types::{PeerKind, PeerRef},
    updates::State,
};
use std::future::Future;

#[derive(Clone)]
pub struct BotUser {
    pub id: u64,
    pub username: String,
    pub active_usernames: Vec<String>,
    pub first_name: String,
    pub has_main_web_app: bool,
    pub inline_enabled: bool,
    pub guest_enabled: bool,
}
#[derive(Clone)]
pub struct Message {
    pub peer: PeerRef,
    pub id: i32,
    pub outgoing: bool,
    pub text: String,
}
pub struct Delivery {
    pub state: State,
    pub event: Option<crate::interactions::Event>,
}
/// Named Telegram operations; callers never construct TL requests.
pub trait TelegramActions: Clone + Send + Sync {
    fn prepare_message(
        &self,
        _user_id: u64,
        _reply: &Reply,
        _launch: Option<&LaunchButton>,
    ) -> impl Future<Output = Result<crate::PreparedMessage, BotError>> + Send {
        async { Err(BotError::Transport) }
    }
    fn media(
        &self,
        _: market_media::MediaKey,
    ) -> impl Future<Output = Result<market_media::MediaAsset, market_media::MediaError>> + Send
    {
        async { Err(market_media::MediaError::Unavailable) }
    }
    fn identity(&self) -> impl Future<Output = Result<BotUser, BotError>> + Send;
    fn configure(
        &self,
        commands: &[MenuCommand],
        launch: Option<&LaunchButton>,
    ) -> impl Future<Output = Result<(), BotError>> + Send;
    fn answer_callback(
        &self,
        _: &crate::interactions::Event,
        _: Option<&str>,
    ) -> impl Future<Output = Result<(), BotError>> + Send {
        async { Ok(()) }
    }
    /// Send an initial progress reply or edit its returned target. Unsupported
    /// transports return None and the dispatcher sends only the final reply.
    fn send_progress(
        &self,
        _event: &crate::interactions::Event,
        _reply: &Reply,
        _launch: Option<&LaunchButton>,
    ) -> impl Future<Output = Result<Option<crate::interactions::Event>, BotError>> + Send {
        async { Ok(None) }
    }
    fn send(
        &self,
        event: &crate::interactions::Event,
        reply: Option<&Reply>,
        launch: Option<&LaunchButton>,
    ) -> impl Future<Output = Result<(), BotError>> + Send;
}

/// Update delivery and acknowledgement remain separate from Telegram actions.
pub trait UpdateCheckpoint {
    fn acknowledge(&self, state: State) -> impl Future<Output = Result<(), BotError>> + Send;
}
pub trait Transport: Send {
    type Actions: TelegramActions + UpdateCheckpoint;
    fn actions(&self) -> Self::Actions;
    fn next(&mut self) -> impl Future<Output = Result<Delivery, BotError>> + Send;
}

pub fn incoming(message: &Message) -> IncomingMessage<'_> {
    IncomingMessage {
        private: message.peer.id.kind() == PeerKind::User,
        outgoing: message.outgoing,
        user_id: (message.peer.id.kind() == PeerKind::User)
            .then(|| {
                message
                    .peer
                    .id
                    .bare_id()
                    .and_then(|id| u64::try_from(id).ok())
            })
            .flatten(),
        message_id: message.id,
        text: Some(&message.text),
    }
}
