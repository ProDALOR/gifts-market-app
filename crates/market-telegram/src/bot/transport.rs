//! Native bot actions and update transport contracts, without TL request types.
use super::{BotError, commands};
use grammers_session::{
    types::{PeerKind, PeerRef},
    updates::State,
};
use std::future::Future;
use url::Url;

pub struct BotUser {
    pub id: u64,
    pub username: String,
    pub active_usernames: Vec<String>,
    pub first_name: String,
    pub has_main_web_app: bool,
}
#[derive(Clone)]
pub(super) struct Message {
    pub peer: PeerRef,
    pub id: i32,
    pub outgoing: bool,
    pub text: String,
}
pub(super) struct Delivery {
    pub state: State,
    pub message: Option<Message>,
}
/// Named Telegram operations; callers never construct TL requests.
pub(super) trait TelegramActions: Clone + Send + Sync {
    type GiftSource: super::GiftSource + Send;
    fn gift_source(&self, message: &Message) -> Self::GiftSource;
    fn identity(&self) -> impl Future<Output = Result<BotUser, BotError>> + Send;
    fn configure(&self, url: Option<&Url>) -> impl Future<Output = Result<(), BotError>> + Send;
    fn send(
        &self,
        message: &Message,
        reply: &commands::Reply,
        url: Option<&Url>,
    ) -> impl Future<Output = Result<(), BotError>> + Send;
}

/// Update delivery and acknowledgement remain separate from Telegram actions.
pub(super) trait UpdateCheckpoint {
    fn acknowledge(&self, state: State) -> impl Future<Output = Result<(), BotError>> + Send;
}
pub(super) trait Transport: Send {
    type Actions: TelegramActions + UpdateCheckpoint;
    fn actions(&self) -> Self::Actions;
    fn next(&mut self) -> impl Future<Output = Result<Delivery, BotError>> + Send;
}

pub(super) fn incoming(message: &Message) -> commands::Incoming<'_> {
    commands::Incoming {
        private: message.peer.id.kind() == PeerKind::User && !message.outgoing,
        user_id: message
            .peer
            .id
            .bare_id()
            .and_then(|id| u64::try_from(id).ok()),
        message_id: message.id,
        text: Some(&message.text),
    }
}
