use crate::*;
use market_telegram::{IncomingMessage, MessageHandler};
fn message(text: &str) -> IncomingMessage<'_> {
    IncomingMessage {
        private: true,
        outgoing: false,
        user_id: Some(7),
        message_id: 1,
        text: Some(text),
    }
}
#[test]
fn gift_detection_accepts_links_slugs_and_short_messages_without_commands() {
    for text in [
        "Gift-123",
        "https://t.me/nft/Gift-123",
        "Please check Gift-123?",
        "Check (https://t.me/nft/Gift-123), please",
        "Gift-123 and gift-00123",
    ] {
        let request = commands::parse(message(text), &["market_bot"]).unwrap();
        let Command::Gift(query) = request.command else {
            panic!("{text}")
        };
        assert_eq!(query.gift.to_string(), "Gift-123");
        assert_eq!(query.currency_id.as_str(), "GRAM");
        assert_eq!(request.user_id, 7);
        assert_eq!(request.message_id, 1);
    }
    for text in ["Gift-123 xtr", "Could you check Gift-123 in XTR?"] {
        let Command::Gift(query) = commands::parse(message(text), &["market_bot"])
            .unwrap()
            .command
        else {
            panic!()
        };
        assert_eq!(query.currency_id.as_str(), "XTR");
    }
    let Command::Gift(query) = commands::parse(message("Gift-123 TON GRAM"), &["market_bot"])
        .unwrap()
        .command
    else {
        panic!()
    };
    assert_eq!(query.currency_id.as_str(), "GRAM");
}
#[test]
fn removed_commands_invalid_and_ambiguous_gifts_are_ignored() {
    for text in [
        "/value",
        "/value Gift-123",
        "/value@market_bot Gift-123",
        "/value@another_bot Gift-123",
        "/floor A",
        "/multiplier",
        "hello",
        "Gift-0",
        "https://evil.example/nft/Gift-123",
        "Gift-123 Other-1",
        "Gift-123 TON XTR",
    ] {
        assert!(
            commands::parse(message(text), &["market_bot"]).is_none(),
            "{text}"
        );
    }
    assert!(
        commands::parse(
            message(&format!("Gift-123 {}", "x".repeat(2048))),
            &["market_bot"]
        )
        .is_none()
    );
}
#[test]
fn parsing_keeps_private_peer_and_bot_target_restrictions() {
    for text in ["/status@market_bot", "Gift-123"] {
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
    assert!(commands::parse(message("/status@another_bot"), &["market_bot"]).is_none());
    assert!(matches!(
        commands::parse(
            message("/status@MARKET_ALIAS"),
            &["market_bot", "market_alias"]
        )
        .unwrap()
        .command,
        Command::Status
    ));
}
#[actix::test]
async fn command_menu_and_help_use_automatic_gift_detection() {
    let handler = commands::LaunchCommands;
    assert_eq!(
        handler
            .commands()
            .iter()
            .map(|c| c.name)
            .collect::<Vec<_>>(),
        ["start", "help"]
    );
    for command in [Command::Start, Command::Help, Command::GiftHelp] {
        let is_help = matches!(command, Command::Help);
        let reply = handler
            .handle(CommandRequest {
                command,
                user_id: 7,
                message_id: 1,
                inline: false,
            })
            .await
            .unwrap();
        assert!(reply.text.contains("gift link or slug"));
        assert_eq!(reply.text.contains("history may be incomplete"), is_help);
        for old in ["/value", "/floor", "/multiplier"] {
            assert!(!reply.text.contains(old));
        }
    }
}

#[test]
fn portfolio_commands_are_bound_to_the_authenticated_sender_or_an_explicit_numeric_subject() {
    for (text, user, currency) in [
        ("/portfolio", 7, "GRAM"),
        ("/portfolio 123", 123, "GRAM"),
        ("/portfolio 123 XTR", 123, "XTR"),
    ] {
        let Command::Portfolio(request) = commands::parse(message(text), &["market_bot"])
            .unwrap()
            .command
        else {
            panic!("{text}")
        };
        assert_eq!(
            (request.user_id.get(), request.currency_id.as_str()),
            (user, currency)
        );
    }
    for text in [
        "/portfolio 0",
        "/portfolio -1",
        "/portfolio 123 GRAM extra",
        "/portfolio@other 123",
    ] {
        assert!(commands::parse(message(text), &["market_bot"]).is_none());
    }
}
