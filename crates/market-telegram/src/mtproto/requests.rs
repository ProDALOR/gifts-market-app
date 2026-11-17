//! TL payload construction stays inside the MTProto adapter.
use crate::{LaunchButton, MenuCommand, Reply, transport::Message};
use grammers_client::tl;
use sha2::{Digest, Sha256};

pub(super) fn command_request(commands: &[MenuCommand]) -> tl::functions::bots::SetBotCommands {
    tl::functions::bots::SetBotCommands {
        scope: tl::enums::BotCommandScope::Default,
        lang_code: String::new(),
        commands: commands
            .iter()
            .map(|command| {
                tl::types::BotCommand {
                    ephemeral: false,
                    command: command.name.into(),
                    description: command.description.into(),
                }
                .into()
            })
            .collect(),
    }
}
pub(super) fn menu_request(launch: &LaunchButton) -> tl::functions::bots::SetBotMenuButton {
    tl::functions::bots::SetBotMenuButton {
        user_id: tl::enums::InputUser::Empty,
        button: tl::types::BotMenuButton {
            text: launch.text.clone(),
            url: launch.url.as_str().into(),
        }
        .into(),
    }
}
pub(super) fn reply_request(
    bot_id: u64,
    message: &Message,
    reply: &Reply,
    launch: Option<&LaunchButton>,
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
        reply_markup: launch.map(|launch| {
            ReplyMarkup::from_buttons_row(&[Button::webview(&launch.text, launch.url.as_str())]).raw
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
