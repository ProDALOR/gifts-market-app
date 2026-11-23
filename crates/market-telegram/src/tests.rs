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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestRequest {
    pub command: String,
    pub user_id: u64,
    pub message_id: i32,
}
pub(crate) fn request(message: IncomingMessage<'_>, usernames: &[String]) -> Option<TestRequest> {
    if !message.private || message.outgoing {
        return None;
    }
    let name = message.text?.split_whitespace().next()?.strip_prefix('/')?;
    let (name, target) = name
        .split_once('@')
        .map_or((name, None), |(name, target)| (name, Some(target)));
    if target.is_some_and(|target| {
        !usernames
            .iter()
            .any(|name| name.eq_ignore_ascii_case(target))
    }) {
        return None;
    }
    Some(TestRequest {
        command: name.into(),
        user_id: message.user_id?,
        message_id: message.message_id,
    })
}
pub(crate) struct TestHandler;
impl MessageHandler for TestHandler {
    type Request = TestRequest;
    fn parse(&self, message: IncomingMessage<'_>, usernames: &[String]) -> Option<TestRequest> {
        request(message, usernames)
    }
    async fn handle(&self, request: TestRequest) -> Result<Reply, BotError> {
        Ok(Reply {
            text: request.command,
            ..Default::default()
        })
    }
}
#[test]
fn replies_are_bounded_in_utf16_without_splitting_characters() {
    let reply = Reply {
        text: "😀".repeat(4096),
        ..Default::default()
    }
    .bounded();
    assert!(reply.text.encode_utf16().count() <= 4096);
    assert!(reply.text.ends_with('…'));
    assert!(
        !Reply {
            text: " ".into(),
            ..Default::default()
        }
        .bounded()
        .text
        .is_empty()
    );
}
