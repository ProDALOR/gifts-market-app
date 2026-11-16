use super::{
    client::rpc,
    requests::{command_request, menu_request, reply_request},
};
use crate::bot::{config::app_url, tests::message, transport::incoming, *};
use grammers_client::{sender::InvocationError, tl};
use grammers_session::types::PeerId;

#[test]
fn mtproto_commands_menu_and_inline_webview_preserve_mini_app_launch() {
    let url = app_url("https://market.example").unwrap();
    let commands = command_request();
    assert!(matches!(
        commands.scope,
        tl::enums::BotCommandScope::Default
    ));
    assert_eq!(
        commands
            .commands
            .iter()
            .map(|command| {
                let tl::enums::BotCommand::Command(command) = command;
                command.command.as_str()
            })
            .collect::<Vec<_>>(),
        ["start", "market", "help", "status", "floor", "portfolio"]
    );
    let menu = menu_request(&url);
    assert!(matches!(menu.user_id, tl::enums::InputUser::Empty));
    let tl::enums::BotMenuButton::Button(menu) = menu.button else {
        panic!("missing menu")
    };
    assert_eq!(menu.url, url.as_str());
    for text in ["/start payload", "/market@MARKET_BOT", "/help"] {
        let message = message(1, text);
        let reply = commands::launch_reply(
            commands::parse(incoming(&message), &["market_bot"])
                .unwrap()
                .command
                .kind(),
        )
        .unwrap();
        let request = reply_request(42, &message, &reply, Some(&url));
        assert_eq!(request.message, reply.text);
        let Some(tl::enums::ReplyMarkup::ReplyInlineMarkup(markup)) = request.reply_markup else {
            panic!("missing inline markup")
        };
        let tl::enums::KeyboardInlineButtonRow::Row(row) = &markup.rows[0];
        let tl::enums::KeyboardInlineButton::Button(button) = &row.buttons[0];
        assert_eq!(button.text, "Open market");
        let tl::enums::InlineButtonType::WebView(webview) = &button.r#type else {
            panic!("missing Mini App button")
        };
        assert_eq!(webview.url, url.as_str());
    }
    for text in ["/start@other_bot", "hello", "/help@other_bot"] {
        assert!(commands::parse(incoming(&message(1, text)), &["market_bot"]).is_none());
    }
    let mut outgoing = message(1, "/start");
    outgoing.outgoing = true;
    assert!(commands::parse(incoming(&outgoing), &["market_bot"]).is_none());
}
#[test]
fn reply_random_id_is_stable_across_retries_and_scoped_to_bot_peer_and_message() {
    let url = app_url("https://market.example").unwrap();
    let mut message = message(1, "/start");
    let reply = commands::launch_reply(
        commands::parse(incoming(&message), &["market_bot"])
            .unwrap()
            .command
            .kind(),
    )
    .unwrap();
    let first = reply_request(42, &message, &reply, Some(&url)).random_id;
    assert_eq!(
        first,
        reply_request(42, &message, &reply, Some(&url)).random_id
    );
    assert_ne!(
        first,
        reply_request(43, &message, &reply, Some(&url)).random_id
    );
    message.id = 2;
    assert_ne!(
        first,
        reply_request(42, &message, &reply, Some(&url)).random_id
    );
    message.id = 1;
    message.peer.id = PeerId::user(8).unwrap();
    assert_ne!(
        first,
        reply_request(42, &message, &reply, Some(&url)).random_id
    );
}
#[test]
fn bot_replies_without_a_miniapp_url_have_no_launch_markup() {
    let reply = Reply {
        text: "Market data: live. Ready to browse.".into(),
    };
    let request = reply_request(42, &message(1, "/status"), &reply, None);
    assert_eq!(request.message, reply.text);
    assert!(request.reply_markup.is_none());
}
#[test]
fn configuration_and_failures_are_sanitized_and_flood_waits_are_honored() {
    for url in [
        "http://market.example",
        "https://user:secret@market.example",
        "https://market.example/?token=secret",
        "https://market.example/#secret",
    ] {
        assert!(app_url(url).is_err());
    }
    for (token, id, hash) in [
        ("secret", 12345, "0123456789abcdef0123456789abcdef"),
        (
            "42:private_test_secret",
            0,
            "0123456789abcdef0123456789abcdef",
        ),
        ("42:private_test_secret", 12345, "secret"),
    ] {
        let error = BotCredentials::new(token.into(), id, hash.into())
            .err()
            .unwrap();
        assert!(!error.to_string().contains("secret"));
    }
    let error = |code, name: &str| {
        InvocationError::Rpc(
            tl::types::RpcError {
                error_code: code,
                error_message: name.into(),
            }
            .into(),
        )
    };
    assert_eq!(
        rpc(error(420, "FLOOD_WAIT_7200")),
        BotError::RetryAfter(7200)
    );
    assert_eq!(BotError::RetryAfter(7200).delay(1).as_secs(), 7200);
    assert_eq!(
        rpc(error(400, "ACCESS_TOKEN_INVALID")),
        BotError::Authorization
    );
    assert_eq!(rpc(error(401, "SESSION_REVOKED")), BotError::Authorization);
    assert_eq!(rpc(error(400, "private_test_secret")), BotError::Rpc(400));
    assert!(
        !rpc(error(400, "private_test_secret"))
            .to_string()
            .contains("secret")
    );
    assert_eq!(
        rpc(error(403, "USER_IS_BLOCKED")),
        BotError::PeerUnavailable
    );
    assert_eq!(rpc(error(400, "BUTTON_TYPE_INVALID")), BotError::Rpc(400));
    assert!(!BotError::Rpc(400).fatal());
    assert_eq!(
        rpc(InvocationError::Dropped("socket request dropped")),
        BotError::Transport
    );
    assert!(BotError::Authorization.fatal());
    assert!(!BotError::Rpc(500).fatal());
}
