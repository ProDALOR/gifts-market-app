//! TL payload construction stays inside the MTProto adapter.
use crate::bot::{commands, transport::Message};
use grammers_client::tl;
use sha2::{Digest, Sha256};
use url::Url;

pub(super) fn command_request() -> tl::functions::bots::SetBotCommands {
    tl::functions::bots::SetBotCommands {
        scope: tl::enums::BotCommandScope::Default,
        lang_code: String::new(),
        commands: commands::CommandKind::ALL
            .iter()
            .map(|command| {
                tl::types::BotCommand {
                    ephemeral: false,
                    command: command.name().into(),
                    description: command.description().into(),
                }
                .into()
            })
            .collect(),
    }
}
pub(super) fn menu_request(url: &Url) -> tl::functions::bots::SetBotMenuButton {
    tl::functions::bots::SetBotMenuButton {
        user_id: tl::enums::InputUser::Empty,
        button: tl::types::BotMenuButton {
            text: "Open market".into(),
            url: url.as_str().into(),
        }
        .into(),
    }
}
pub(super) fn reply_request(
    bot_id: u64,
    message: &Message,
    reply: &commands::Reply,
    url: Option<&Url>,
) -> tl::functions::messages::SendMessage {
    use grammers_client::message::{Button, ReplyMarkup};
    let mut hash = Sha256::new();
    hash.update(b"market-app-bot-reply-v1\0");
    hash.update(bot_id.to_be_bytes());
    hash.update(message.peer.id.bare_id_unchecked().to_be_bytes());
    hash.update(message.id.to_be_bytes());
    let random_id = i64::from_be_bytes(hash.finalize()[..8].try_into().unwrap());
    tl::functions::messages::SendMessage {
        no_webpage: true,
        silent: false,
        background: false,
        clear_draft: false,
        noforwards: false,
        update_stickersets_order: false,
        invert_media: false,
        allow_paid_floodskip: false,
        peer: message.peer.into(),
        reply_to: None,
        message: reply.text.clone(),
        random_id,
        reply_markup: url.map(|url| {
            ReplyMarkup::from_buttons_row(&[Button::webview("Open market", url.as_str())]).raw
        }),
        entities: None,
        schedule_date: None,
        schedule_repeat_period: None,
        send_as: None,
        quick_reply_shortcut: None,
        effect: None,
        allow_paid_stars: None,
        suggested_post: None,
        rich_message: None,
    }
}
