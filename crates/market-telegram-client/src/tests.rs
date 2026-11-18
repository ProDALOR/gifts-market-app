//! Synthetic application handlers keep transport tests independent of market commands and actors.
use crate::transport::Message;
use crate::*;
use grammers_session::types::{PeerAuth, PeerId, PeerRef};
pub(crate) fn credentials() -> BotCredentials {
    BotCredentials::new(
        "42:private_test_secret".into(),
        12345,
        "0123456789abcdef0123456789abcdef".into(),
    )
    .unwrap()
}
pub(crate) fn message(id: i32, text: &str) -> Message {
    Message {
        peer: PeerRef {
            id: PeerId::user(7).unwrap(),
            auth: PeerAuth::from_hash(123),
        },
        id,
        outgoing: false,
        text: text.into(),
    }
}
