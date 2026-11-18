use crate::*;
use market_telegram::{IncomingMessage, MessageHandler};
fn message(text: &str) -> IncomingMessage<'_> {
    IncomingMessage {
        private: true,
        outgoing: false,
        user_id: Some(7),
        message_id: 21,
        text: Some(text),
    }
}
#[test]
fn gift_commands_and_bare_links_share_slug_validation_and_currency_shorthand() {
    for text in [
        "/value@MARKET_BOT Gift-123 TON",
        "/value https://t.me/nft/Gift-123",
        "https://t.me/nft/Gift-123",
        "Gift-00123",
    ] {
        let request = commands::parse(message(text), &["market_bot"]).unwrap();
        assert_eq!(request.user_id, 7);
        assert_eq!(request.message_id, 21);
        let Command::Value(query) = request.command else {
            panic!("expected gift valuation");
        };
        assert_eq!(query.gift.to_string(), "Gift-123");
        assert_eq!(query.currency_id.as_str(), "GRAM");
    }
    let request = commands::parse(message("/value Gift-123 xtr"), &["market_bot"]).unwrap();
    let Command::Value(query) = request.command else {
        panic!();
    };
    assert_eq!(query.currency_id.as_str(), "XTR");
}
#[test]
fn invalid_gift_commands_and_removed_commands_do_not_invoke_valuation() {
    for text in [
        "/value",
        "/value Gift-0",
        "/value https://evil.example/nft/Gift-123",
        "/value Gift-123 GRAM user_id=9",
        "/value Gift-123 | GRAM",
    ] {
        assert!(
            matches!(
                commands::parse(message(text), &["market_bot"])
                    .unwrap()
                    .command,
                Command::Invalid(CommandKind::Value)
            ),
            "{text}"
        );
    }
    for text in [
        "/floor model_floor | A | B",
        "/portfolio",
        "/multiplier",
        "hello",
        "/value@another_bot Gift-123",
    ] {
        assert!(
            commands::parse(message(text), &["market_bot"]).is_none(),
            "{text}"
        );
    }
}
#[test]
fn parsing_keeps_private_peer_and_bot_target_restrictions() {
    for text in ["/value Gift-123", "Gift-123"] {
        let mut incoming = message(text);
        incoming.private = false;
        assert!(commands::parse(incoming, &["market_bot"]).is_none());
        incoming = message(text);
        incoming.outgoing = true;
        assert!(commands::parse(incoming, &["market_bot"]).is_none());
        incoming = message(text);
        incoming.user_id = None;
        assert!(commands::parse(incoming, &["market_bot"]).is_none());
    }
}
#[actix::test]
async fn command_menu_and_help_only_advertise_gift_valuation() {
    let handler = commands::LaunchCommands;
    let menu: Vec<_> = handler.commands().iter().map(|c| c.name).collect();
    assert_eq!(menu, ["start", "market", "help", "status", "value"]);
    let help = handler
        .handle(CommandRequest {
            command: Command::Help,
            user_id: 7,
            message_id: 1,
        })
        .await
        .unwrap();
    assert!(help.text.contains("/value"));
    for old in ["/portfolio", "/floor", "/multiplier"] {
        assert!(!help.text.contains(old));
    }
    let invalid = handler
        .handle(CommandRequest {
            command: Command::Invalid(CommandKind::Value),
            user_id: 7,
            message_id: 1,
        })
        .await
        .unwrap();
    assert!(invalid.text.contains("/value"));
}
