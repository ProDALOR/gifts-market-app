//! Telegram interaction identities, with no TL payloads in application handlers.
use crate::transport::Message;
use grammers_session::types::PeerId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InlineMessageId {
    Standard {
        dc_id: i32,
        id: i64,
        access_hash: i64,
    },
    Wide {
        dc_id: i32,
        owner_id: i64,
        id: i32,
        access_hash: i64,
    },
}
#[derive(Clone)]
pub enum CallbackTarget {
    Message(Message),
    Inline(InlineMessageId),
}
#[derive(Clone)]
pub enum Event {
    Message(Message),
    Inline {
        id: i64,
        user_id: u64,
        query: String,
        offset: String,
    },
    Guest {
        id: i64,
        user_id: u64,
        text: String,
        references: Vec<String>,
    },
    Callback {
        id: i64,
        user_id: u64,
        data: Vec<u8>,
        target: CallbackTarget,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Lane {
    Chat(PeerId),
    InlineUser(u64),
    Guest(i64),
    InlineMessage(InlineMessageId),
}
impl Event {
    pub fn lane(&self) -> Lane {
        match self {
            Self::Message(message) => Lane::Chat(message.peer.id),
            Self::Inline { user_id, .. } => Lane::InlineUser(*user_id),
            Self::Guest { id, .. } => Lane::Guest(*id),
            Self::Callback {
                target: CallbackTarget::Message(message),
                ..
            } => Lane::Chat(message.peer.id),
            Self::Callback {
                target: CallbackTarget::Inline(id),
                ..
            } => Lane::InlineMessage(id.clone()),
        }
    }
    pub fn inline_user(&self) -> Option<u64> {
        if let Self::Inline { user_id, .. } = self {
            Some(*user_id)
        } else {
            None
        }
    }
}
