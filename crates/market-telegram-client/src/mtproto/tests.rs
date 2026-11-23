use super::{
    client::rpc,
    requests::{command_request, menu_request, reply_request},
};
use crate::{config::app_url, tests::message, *};
use grammers_client::{sender::InvocationError, tl};
use grammers_session::types::PeerId;

#[test]
fn mtproto_configuration_and_webview_use_application_supplied_menu_and_label() {
    let launch = LaunchButton {
        text: "Open test app".into(),
        url: app_url("https://market.example").unwrap(),
    };
    let commands = command_request(&[MenuCommand {
        name: "test",
        description: "Custom handler",
    }]);
    assert!(matches!(
        commands.scope,
        tl::enums::BotCommandScope::Default
    ));
    let tl::enums::BotCommand::Command(command) = &commands.commands[0];
    assert_eq!(command.command, "test");
    assert_eq!(command.description, "Custom handler");
    let tl::enums::BotMenuButton::Button(menu) = menu_request(&launch).button else {
        panic!("missing menu")
    };
    assert_eq!(menu.text, launch.text);
    assert_eq!(menu.url, launch.url.as_str());
    let request = reply_request(
        42,
        &message(1, "hello"),
        &Reply {
            text: "reply".into(),
            ..Default::default()
        },
        Some(&launch),
    );
    let Some(tl::enums::ReplyMarkup::ReplyInlineMarkup(markup)) = request.reply_markup else {
        panic!("missing markup")
    };
    let tl::enums::KeyboardInlineButtonRow::Row(row) = &markup.rows[0];
    let tl::enums::KeyboardInlineButton::Button(button) = &row.buttons[0];
    assert_eq!(button.text, launch.text);
    let tl::enums::InlineButtonType::WebView(webview) = &button.r#type else {
        panic!("missing webview")
    };
    assert_eq!(webview.url, launch.url.as_str());
}
#[test]
fn reply_random_id_is_stable_across_retries_and_scoped_to_bot_peer_and_message() {
    let launch = LaunchButton {
        text: "Open test app".into(),
        url: app_url("https://market.example").unwrap(),
    };
    let mut message = message(1, "/start");
    let reply = Reply {
        text: "reply".into(),
        ..Default::default()
    };
    let first = reply_request(42, &message, &reply, Some(&launch)).random_id;
    assert_eq!(
        first,
        reply_request(42, &message, &reply, Some(&launch)).random_id
    );
    assert_ne!(
        first,
        reply_request(43, &message, &reply, Some(&launch)).random_id
    );
    message.id = 2;
    assert_ne!(
        first,
        reply_request(42, &message, &reply, Some(&launch)).random_id
    );
    message.id = 1;
    message.peer.id = PeerId::user(8).unwrap();
    assert_ne!(
        first,
        reply_request(42, &message, &reply, Some(&launch)).random_id
    );
}
#[test]
fn bot_replies_without_a_miniapp_url_have_no_launch_markup() {
    let reply = Reply {
        text: "Market data: live. Ready to browse.".into(),
        ..Default::default()
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

/// Read-only inspection of public emoji metadata. Writes no session/file references or credentials.
#[tokio::test]
#[ignore = "requires explicit bot credentials and public emoji pack/document inputs"]
async fn inspect_live_public_emoji_packs() {
    use super::invocations::Group;
    use crate::transport::{TelegramActions, Transport};
    use market_invocations::{Priority, Replay};
    let credentials = BotCredentials::new(
        std::env::var("TELEGRAM_BOT_TOKEN").unwrap(),
        std::env::var("TELEGRAM_API_ID").unwrap().parse().unwrap(),
        std::env::var("TELEGRAM_API_HASH").unwrap(),
    )
    .unwrap();
    assert_eq!(
        credentials.bot_id(),
        std::env::var("MARKET_TEST_BOT_ID")
            .unwrap()
            .parse::<u64>()
            .unwrap()
    );
    let directory = tempfile::tempdir().unwrap();
    let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
    let connection = super::Connection::open(credentials, directory.path(), scheduler.handle())
        .await
        .unwrap();
    let actions = connection.transport.actions();
    actions.identity().await.unwrap();
    let ids = std::env::var("MARKET_TEST_EMOJI_IDS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<i64>().unwrap())
        .collect::<Vec<_>>();
    let names = std::env::var("MARKET_TEST_EMOJI_PACKS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(ids.len() <= 32 && names.len() <= 12);
    fn document(raw: &tl::enums::Document) -> serde_json::Value {
        let tl::enums::Document::Document(doc) = raw else {
            return serde_json::json!({"id": raw.id()});
        };
        let custom = doc.attributes.iter().find_map(|a| match a {
            tl::enums::DocumentAttribute::CustomEmoji(a) => Some(serde_json::json!({"alternative": a.alt, "text_color": a.text_color, "free": a.free})),
            _ => None,
        });
        serde_json::json!({"id": doc.id.to_string(), "mime": doc.mime_type, "size": doc.size, "custom_emoji": custom})
    }
    let mut output = serde_json::json!({"documents": [], "packs": []});
    let mut sets = Vec::new();
    for id in ids {
        let docs = actions
            .invocations
            .rpc(
                &actions.client,
                tl::functions::messages::GetCustomEmojiDocuments {
                    document_id: vec![id],
                },
                Group::Control,
                Priority::Background,
                Replay::Safe,
            )
            .await
            .unwrap();
        for raw in docs {
            output["documents"]
                .as_array_mut()
                .unwrap()
                .push(document(&raw));
            if let tl::enums::Document::Document(doc) = raw {
                for attribute in doc.attributes {
                    if let tl::enums::DocumentAttribute::CustomEmoji(a) = attribute
                        && !sets.contains(&a.stickerset)
                    {
                        sets.push(a.stickerset);
                    }
                }
            }
        }
    }
    for name in names {
        sets.push(tl::types::InputStickerSetShortName { short_name: name }.into());
    }
    let mut sampled = 0;
    for stickerset in sets {
        match actions
            .invocations
            .rpc(
                &actions.client,
                tl::functions::messages::GetStickerSet {
                    stickerset,
                    hash: 0,
                },
                Group::Control,
                Priority::Background,
                Replay::Safe,
            )
            .await
        {
            Ok(tl::enums::messages::StickerSet::Set(set)) => {
                let tl::enums::StickerSet::Set(info) = set.set;
                if let Ok(directory) = std::env::var("MARKET_TEST_EMOJI_ASSET_DIR") {
                    std::fs::create_dir_all(&directory).unwrap();
                    for raw in &set.documents {
                        let tl::enums::Document::Document(doc) = raw else {
                            continue;
                        };
                        if sampled >= 10
                            || doc.mime_type != "image/webp"
                            || !(1..=32768).contains(&doc.size)
                        {
                            continue;
                        }
                        let path =
                            std::path::Path::new(&directory).join(format!("{}.webp", doc.id));
                        let media = grammers_client::media::Document::from_raw_media(
                            tl::types::MessageMediaDocument {
                                document: Some(raw.clone()),
                                nopremium: false,
                                spoiler: false,
                                video: false,
                                round: false,
                                voice: false,
                                alt_documents: None,
                                video_cover: None,
                                video_timestamp: None,
                                ttl_seconds: None,
                            },
                        );
                        let client = actions.client.clone();
                        actions
                            .invocations
                            .run(
                                Group::Media,
                                Priority::Background,
                                Replay::Safe,
                                move || {
                                    let client = client.clone();
                                    let media = media.clone();
                                    let path = path.clone();
                                    async move { client.download_media(&media, path).await }
                                },
                            )
                            .await
                            .map_err(|error| error.bot())
                            .unwrap();
                        sampled += 1;
                    }
                }
                let keywords = set
                    .keywords
                    .into_iter()
                    .map(|k| {
                        let tl::enums::StickerKeyword::Keyword(k) = k;
                        serde_json::json!({"id": k.document_id.to_string(), "keywords": k.keyword})
                    })
                    .collect::<Vec<_>>();
                output["packs"].as_array_mut().unwrap().push(serde_json::json!({"name": info.short_name, "title": info.title, "emojis": info.emojis, "documents": set.documents.iter().map(document).collect::<Vec<_>>(), "keywords": keywords}));
            }
            _ => eprintln!("A requested public emoji pack was unavailable"),
        }
    }
    std::fs::write(
        std::env::var("MARKET_TEST_EMOJI_OUTPUT").unwrap(),
        serde_json::to_vec_pretty(&output).unwrap(),
    )
    .unwrap();
    connection.close().await.unwrap();
    scheduler.shutdown().await;
    eprintln!(
        "Inspected {} documents and {} public packs",
        output["documents"].as_array().unwrap().len(),
        output["packs"].as_array().unwrap().len()
    );
}
