use super::{
    transport::{Message, incoming},
    *,
};
use grammers_session::types::{PeerAuth, PeerId, PeerRef};

pub(super) fn credentials() -> BotCredentials {
    BotCredentials::new(
        "42:private_test_secret".into(),
        12345,
        "0123456789abcdef0123456789abcdef".into(),
    )
    .unwrap()
}
pub(super) fn message(id: i32, text: &str) -> Message {
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
#[test]
fn floor_and_portfolio_arguments_are_typed_without_overriding_caller_identity() {
    let request = commands::parse(
        incoming(&message(
            21,
            "/floor@MARKET_BOT Bunny Muffin | Red Alert | TON",
        )),
        &["market_bot"],
    )
    .unwrap();
    assert_eq!(request.user_id, 7);
    let Command::Floor(query) = request.command else {
        panic!("expected floor")
    };
    assert_eq!(query.collection, "Bunny Muffin");
    assert_eq!(query.model, "Red Alert");
    assert_eq!(query.currency.as_str(), "GRAM");
    let request =
        commands::parse(incoming(&message(22, "/portfolio TON")), &["market_bot"]).unwrap();
    assert_eq!(request.user_id, 7);
    let Command::Portfolio(currency) = request.command else {
        panic!("expected portfolio")
    };
    assert_eq!(currency.as_str(), "GRAM");
    for command in [
        "/floor bad",
        "/floor A | B | GRAM | XTR",
        "/portfolio GRAM user_id=999",
    ] {
        let request = commands::parse(incoming(&message(23, command)), &["market_bot"]).unwrap();
        assert!(matches!(request.command, Command::Invalid(_)));
        assert_eq!(request.user_id, 7);
    }
}
#[test]
fn replies_are_bounded_in_utf16_without_splitting_characters() {
    let reply = Reply {
        text: "😀".repeat(4096),
    }
    .bounded();
    assert!(reply.text.encode_utf16().count() <= 4096);
    assert!(reply.text.ends_with('…'));
    assert!(!Reply { text: " ".into() }.bounded().text.is_empty());
}
