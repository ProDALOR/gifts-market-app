//! Explicit opt-in live probes. Query probes do not send messages; edit probes
//! require an explicit authorized recipient. Neither acknowledges bot updates.
use super::*;
use crate::mtproto::{interactions::inline_result, requests::inline_request};
use crate::{
    BotCredentials, Connection, Reply,
    transport::{TelegramActions, Transport},
};
use grammers_session::updates::UpdatesLike;

fn update_rows(packet: UpdatesLike) -> Vec<tl::enums::Update> {
    match packet {
        UpdatesLike::Updates(tl::enums::Updates::UpdateShort(packet)) => vec![packet.update],
        UpdatesLike::Updates(tl::enums::Updates::Updates(packet)) => packet.updates,
        UpdatesLike::Updates(tl::enums::Updates::Combined(packet)) => packet.updates,
        _ => vec![],
    }
}

async fn matching_query(
    updates: &mut tokio::sync::mpsc::Receiver<UpdatesLike>,
    user: i64,
    marker: &str,
) -> Result<i64, BotError> {
    while let Some(packet) = updates.recv().await {
        for row in update_rows(packet) {
            if let tl::enums::Update::BotInlineQuery(query) = row
                && query.user_id == user
                && query.query == marker
                && query.offset.is_empty()
            {
                return Ok(query.query_id);
            }
        }
    }
    Err(BotError::Runner)
}

async fn guest_probe(
    user: &Client,
    chat: grammers_session::types::PeerRef,
    user_id: i64,
    actions: &crate::mtproto::client::MtProtoActions,
    updates: &mut tokio::sync::mpsc::Receiver<UpdatesLike>,
    identity: &crate::BotUser,
    reply: &Reply,
) -> anyhow::Result<()> {
    use crate::interactions::Event;
    let prepared = actions.prepare_rich(Some(reply)).await;
    let Some(tl::enums::InputRichMessage::Html(html)) = prepared else {
        anyhow::bail!("guest probe requires a rich response")
    };
    anyhow::ensure!(
        html.files.is_some_and(|files| files.len() == 1),
        "guest probe needs one cover"
    );
    actions
        .invocations
        .rpc_call(
            &actions.client,
            tl::functions::updates::GetState {},
            Group::Control,
            Priority::Interactive,
            Replay::Safe,
        )
        .await
        .map_err(CallError::bot)?;
    let text = format!(
        "@{} SakuraFlower-59197 guestprobe{}",
        identity.username,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let message = grammers_client::message::InputMessage::new()
        .text(text.clone())
        .fmt_entities([tl::types::MessageEntityMention {
            offset: 0,
            length: (identity.username.len() + 1).try_into()?,
        }
        .into()])
        .link_preview(false);
    let sent = user
        .send_message(chat, message)
        .await
        .map_err(crate::mtproto::client::rpc)?;
    eprintln!("LIVE GUEST: posted unique mention in the explicitly selected test chat");
    let event = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(packet) = updates.recv().await {
            for row in update_rows(packet) {
                if let tl::enums::Update::BotGuestChatQuery(query) = row {
                    if let tl::enums::Message::Message(message) = &query.message
                        && message.message == text {
                        eprintln!("LIVE GUEST: matching raw guest update; outgoing={}, has_sender={}", message.out, message.from_id.is_some());
                    }
                    if let Some(event) = crate::mtproto::client::guest_event(query)
                        && matches!(&event, Event::Guest { user_id: from, text: content, .. } if *from == user_id as u64 && content == &text)
                    { return Ok::<_, BotError>(event); }
                }
            }
        }
        Err(BotError::Runner)
    }).await??;
    eprintln!("LIVE GUEST: received matching native guest query");
    actions.send(&event, Some(reply), None).await?;
    eprintln!("LIVE GUEST: Telegram accepted rich response with cover");
    for _ in 0..5 {
        let mut messages = user.iter_messages(chat).limit(5);
        while let Some(message) = messages.next().await.map_err(crate::mtproto::client::rpc)? {
            if let tl::enums::Message::Message(message) = message.raw
                && message.id > sent.id()
                && matches!(message.from_id, Some(tl::enums::Peer::User(from)) if from.user_id == identity.id as i64)
                && matches!(message.guestchat_via_from, Some(tl::enums::Peer::User(from)) if from.user_id == user_id)
            {
                anyhow::ensure!(
                    message.rich_message.is_some(),
                    "guest response lost rich formatting"
                );
                eprintln!(
                    "LIVE GUEST: verified bot's rich guest message in the selected test chat"
                );
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("guest response not found in the selected test chat")
}

fn captured_reply(path: &str) -> Reply {
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    Reply {
        text: data["text"].as_str().unwrap().into(),
        title: data["title"].as_str().map(str::to_owned),
        summary: data["summary"].as_str().map(str::to_owned),
        rich_html: data["rich_html"].as_str().map(str::to_owned),
        artwork: data["artwork"]
            .as_object()
            .map(|cover| market_media::GiftCover {
                model: market_media::DocumentId::new(cover["model"].as_i64().unwrap()).unwrap(),
                pattern: cover["pattern"]
                    .as_i64()
                    .map(|id| market_media::DocumentId::new(id).unwrap()),
                palette: cover["palette"]
                    .as_array()
                    .map(|rgb| market_media::CoverPalette {
                        center: rgb[0].as_u64().unwrap().try_into().unwrap(),
                        edge: rgb[1].as_u64().unwrap().try_into().unwrap(),
                        pattern: rgb[2].as_u64().unwrap().try_into().unwrap(),
                    }),
            }),
        ..Default::default()
    }
    .bounded()
}

#[tokio::test]
#[ignore = "sends and edits one report with an explicitly authorized bot and recipient"]
async fn existing_bot_accepts_rich_message_edits() {
    use crate::interactions::{CallbackTarget, Event};
    use grammers_session::types::{PeerAuth, PeerId, PeerRef};
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
    let recipient = std::env::var("MARKET_TEST_RECIPIENT")
        .unwrap()
        .parse::<i64>()
        .unwrap();
    let directory = PathBuf::from(std::env::var("MARKET_TEST_SESSION_DIR").unwrap());
    assert!(
        directory.is_absolute(),
        "use the existing bot session's absolute directory; Cargo runs tests from the crate directory"
    );
    assert!(
        directory
            .join(format!(
                "{}-{}.mtproto.sqlite",
                credentials.bot_id(),
                credentials.api_id
            ))
            .is_file(),
        "the edit probe must reuse an existing bot session"
    );

    let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
    let mut bot = Connection::open(credentials.clone(), &directory, scheduler.handle())
        .await
        .unwrap();
    let (cache, mut downloader) = market_media::MediaCache::open(PathBuf::from(
        std::env::var("MARKET_TEST_MEDIA_DIR").unwrap(),
    ))
    .await
    .unwrap();
    bot.transport.bind_artwork(cache.clone());
    let actions = bot.transport.actions();
    let media = actions.clone();
    let downloads = tokio::spawn(async move {
        downloader.run(|key| media.media(key)).await;
    });
    let result: anyhow::Result<()> = async {
        assert_eq!(actions.identity().await?.id, credentials.bot_id());
        let floors = captured_reply(&std::env::var("MARKET_TEST_FLOORS_REPLY").unwrap());
        let sales = captured_reply(&std::env::var("MARKET_TEST_SALES_REPLY").unwrap());
        let mut message = crate::transport::Message {
            peer: PeerRef {
                id: PeerId::user(recipient).unwrap(),
                auth: PeerAuth::from_hash(0),
            },
            id: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .subsec_nanos()
                & 0x7fffffff) as i32,
            outgoing: false,
            text: String::new(),
        };
        let mut request =
            crate::mtproto::requests::reply_request(credentials.bot_id(), &message, &floors, None);
        request.rich_message = actions.prepare_rich(Some(&floors)).await;
        anyhow::ensure!(
            request.rich_message.is_some(),
            "probe requires rich content"
        );
        let updates = actions
            .invocations
            .rpc_call(
                &actions.client,
                request,
                Group::Reply(recipient),
                Priority::Interactive,
                Replay::Deduplicated,
            )
            .await
            .map_err(CallError::bot)?;
        message.id = match updates {
            tl::enums::Updates::UpdateShortSentMessage(sent) => sent.id,
            updates => {
                let rows = match updates {
                    tl::enums::Updates::Updates(updates) => updates.updates,
                    tl::enums::Updates::Combined(updates) => updates.updates,
                    _ => anyhow::bail!("unexpected send response"),
                };
                rows.into_iter()
                    .find_map(|update| match update {
                        tl::enums::Update::NewMessage(update) => match update.message {
                            tl::enums::Message::Message(message) => Some(message.id),
                            _ => None,
                        },
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("sent message ID missing"))?
            }
        };
        eprintln!("LIVE EDIT: sent one report to the explicitly authorized recipient");
        let event = Event::Callback {
            id: 0,
            user_id: recipient as u64,
            data: vec![],
            target: CallbackTarget::Message(message),
        };
        actions.send(&event, Some(&floors), None).await?;
        actions.send(&event, Some(&sales), None).await?;
        eprintln!("LIVE EDIT: corrected Floors / Sales edits both accepted");
        Ok(())
    }
    .await;
    cache.shutdown().await;
    let _ = downloads.await;
    let closed = bot.close().await;
    scheduler.shutdown().await;
    closed.unwrap();
    result.unwrap();
}

#[tokio::test]
#[ignore = "uses explicitly authorized existing Telegram user and bot sessions for live inline queries"]
async fn existing_sessions_accept_patched_inline_answers() {
    let legacy_offset = std::env::var("MARKET_TEST_LEGACY_INLINE_OFFSET").is_ok_and(|v| v == "1");
    let encoding = if legacy_offset {
        "legacy-empty-offset"
    } else {
        "patched"
    };
    let expected_bot: u64 = std::env::var("MARKET_TEST_BOT_ID")
        .unwrap()
        .parse()
        .unwrap();
    let expected_user = std::env::var("MARKET_TEST_USER_ID")
        .ok()
        .map(|id| id.parse::<i64>().unwrap());
    let credentials = BotCredentials::new(
        std::env::var("TELEGRAM_BOT_TOKEN").unwrap(),
        std::env::var("TELEGRAM_API_ID").unwrap().parse().unwrap(),
        std::env::var("TELEGRAM_API_HASH").unwrap(),
    )
    .unwrap();
    assert_eq!(credentials.bot_id(), expected_bot);
    let bytes = std::fs::read(std::env::var("MARKET_TEST_ACCOUNTS").unwrap()).unwrap();
    let accounts = parse_accounts(&bytes).unwrap();
    assert_eq!(
        accounts.len(),
        1,
        "explicitly select one test account export"
    );
    let account = &accounts[0];
    let directory = PathBuf::from(std::env::var("MARKET_TEST_SESSION_DIR").unwrap());
    std::fs::create_dir_all(directory.join("accounts")).unwrap();
    let path = directory.join("accounts").join(format!(
        "account-{:x}.lock",
        Sha256::digest(account.auth_key)
    ));
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(path).unwrap();
    lock.try_lock()
        .expect("existing account must not have another local worker");
    let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
    let mut user = UserConnection::open(account).await.unwrap();
    let mut bot = Connection::open(credentials, &directory, scheduler.handle())
        .await
        .unwrap();
    let (cache, mut downloader) = market_media::MediaCache::open(PathBuf::from(
        std::env::var("MARKET_TEST_MEDIA_DIR").unwrap(),
    ))
    .await
    .unwrap();
    bot.transport.bind_artwork(cache.clone());
    let actions = bot.transport.actions();
    let media = actions.clone();
    let downloads = tokio::spawn(async move {
        downloader.run(|key| media.media(key)).await;
    });
    let mut updates = bot.transport.take_test_updates();
    let result: anyhow::Result<()> = async {
        let identity = actions.identity().await?;
        anyhow::ensure!(identity.id == expected_bot && identity.inline_enabled, "unexpected bot or inline disabled");
        let me = user.client.get_me().await.map_err(crate::mtproto::client::rpc)?;
        let user_id = me.id().bare_id().filter(|id| *id > 0).ok_or(BotError::InvalidResponse)?;
        anyhow::ensure!(!me.is_bot() && expected_user.is_none_or(|expected| expected == user_id), "unexpected user session");
        let peer = user.client.resolve_username(&identity.username).await.map_err(crate::mtproto::client::rpc)?
            .ok_or(BotError::InvalidResponse)?;
        let input: tl::enums::InputUser = peer.to_ref().await.map_err(|_| BotError::Session)?.ok_or(BotError::InvalidResponse)?.into();
        let floors = captured_reply(&std::env::var("MARKET_TEST_FLOORS_REPLY").unwrap());
        if std::env::var("MARKET_TEST_GUEST_MODE").is_ok_and(|v| v == "1") {
            anyhow::ensure!(identity.guest_enabled, "Guest Mode is disabled");
            let target = std::env::var("MARKET_TEST_GUEST_CHAT").expect("explicitly authorized guest test destination");
            let chat = if target == "self" {
                me.to_ref().await.map_err(|_| BotError::Session)?.ok_or(BotError::InvalidResponse)?
            } else {
                user.client.resolve_username(target.trim_start_matches('@')).await.map_err(crate::mtproto::client::rpc)?
                    .ok_or(BotError::InvalidResponse)?.to_ref().await.map_err(|_| BotError::Session)?.ok_or(BotError::InvalidResponse)?
            };
            return guest_probe(&user.client, chat, user_id, &actions, &mut updates, &identity, &floors).await;
        }
        let sales = captured_reply(&std::env::var("MARKET_TEST_SALES_REPLY").unwrap());
        let plain = Reply::text("Inline transport live probe");
        for (label, reply) in [("plain", Some(&plain)), ("floors", Some(&floors)), ("sales", Some(&sales)), ("empty", None)] {
            let prepared = actions.prepare_rich(reply).await;
            if label == "floors" || label == "sales" {
                let Some(tl::enums::InputRichMessage::Html(html)) = &prepared else { anyhow::bail!("rich probe lost markup") };
                anyhow::ensure!(html.files.as_ref().is_some_and(|files| files.len() == 1), "rich probe must contain one cover");
            }
            let results = reply.map(|r| vec![inline_result(r, None, None, prepared, true)]).unwrap_or_default();
            let marker = format!("/__market_inline_probe_{}_{}", label, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos());
            let query = tl::functions::messages::GetInlineBotResults {
                bot: input.clone(), peer: tl::enums::InputPeer::PeerSelf,
                geo_point: None, query: marker.clone(), offset: String::new(),
            };
            let request = user.client.invoke(&query);
            let answer = async {
                let id = matching_query(&mut updates, user_id, &marker).await?;
                let mut request = inline_request(id, results);
                if legacy_offset { request.next_offset = Some(String::new()); }
                let outcome = actions.invocations.rpc_call(&actions.client, request, Group::Inline, Priority::Interactive, Replay::Safe).await;
                if legacy_offset && matches!(&outcome, Err(CallError::Remote(grammers_client::sender::InvocationError::Rpc(error))) if error.code == 400 && error.name == "NEXT_OFFSET_INVALID") {
                    eprintln!("LIVE INLINE baseline: reproduced expected 400 NEXT_OFFSET_INVALID with empty offset");
                    return Ok(None);
                }
                let accepted = outcome.map_err(|error| {
                    if let CallError::Remote(grammers_client::sender::InvocationError::Rpc(rpc)) = &error {
                        let name = match rpc.name.as_str() {
                            "NEXT_OFFSET_INVALID" | "QUERY_ID_INVALID" | "RICH_MESSAGE_INVALID" | "SEND_MESSAGE_TYPE_INVALID" => rpc.name.as_str(),
                            _ => "OTHER_RPC_ERROR",
                        };
                        eprintln!("LIVE INLINE {label}: rejected with {} {name}", rpc.code);
                    }
                    error.bot()
                })?;
                anyhow::ensure!(accepted, "Telegram did not accept inline answer");
                Ok::<_, anyhow::Error>(Some(id))
            };
            let (received, answered) = tokio::time::timeout(Duration::from_secs(20), async { tokio::join!(request, answer) }).await?;
            let Some(answered) = answered? else { return Ok(()) };
            let tl::enums::messages::BotResults::Results(received) = received.map_err(crate::mtproto::client::rpc)?;
            anyhow::ensure!(received.query_id == answered, "received a different query's answer");
            anyhow::ensure!(received.next_offset.as_deref().is_none_or(str::is_empty), "unexpected inline pagination");
            anyhow::ensure!(received.results.len() == usize::from(reply.is_some()), "another worker answered this probe");
            if let Some(reply) = reply {
                let tl::enums::BotInlineResult::Result(result) = &received.results[0] else { anyhow::bail!("expected article result") };
                if label == "plain" {
                    let tl::enums::BotInlineMessage::Text(message) = &result.send_message else { anyhow::bail!("plain answer changed type") };
                    anyhow::ensure!(message.message == reply.text, "another worker answered plain probe");
                } else {
                    anyhow::ensure!(result.title == reply.title, "another worker answered rich probe");
                    anyhow::ensure!(matches!(result.send_message, tl::enums::BotInlineMessage::RichMessage(_)), "rich response fell back to text");
                }
            }
            eprintln!("LIVE INLINE {label}: Telegram accepted {encoding} request; user received expected result");
        }
        Ok(())
    }.await;
    cache.shutdown().await;
    let _ = downloads.await;
    let closed = bot.close().await;
    user.close().await;
    scheduler.shutdown().await;
    closed.unwrap();
    result.unwrap();
}
