//! TL payload construction stays inside the MTProto adapter.
use crate::{LaunchButton, MenuCommand, Reply, transport::Message};
use grammers_client::tl;
use sha2::{Digest, Sha256};

pub(super) fn chat_edit_request(
    message: &Message,
    reply: &Reply,
    rich: Option<tl::enums::InputRichMessage>,
    markup: Option<tl::enums::ReplyMarkup>,
) -> tl::functions::messages::EditMessage {
    tl::functions::messages::EditMessage {
        no_webpage: true,
        invert_media: false,
        peer: message.peer.into(),
        id: message.id,
        message: rich.is_none().then(|| reply.text.clone()),
        media: None,
        reply_markup: markup,
        entities: None,
        schedule_date: None,
        schedule_repeat_period: None,
        quick_reply_shortcut_id: None,
        rich_message: rich,
    }
}
pub(super) fn inline_edit_request(
    id: tl::enums::InputBotInlineMessageId,
    reply: &Reply,
    rich: Option<tl::enums::InputRichMessage>,
    markup: Option<tl::enums::ReplyMarkup>,
) -> tl::functions::messages::EditInlineBotMessage {
    tl::functions::messages::EditInlineBotMessage {
        no_webpage: true,
        invert_media: false,
        id,
        message: rich.is_none().then(|| reply.text.clone()),
        media: None,
        reply_markup: markup,
        entities: None,
        rich_message: rich,
    }
}

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
pub(super) fn inline_request(
    query_id: i64,
    results: Vec<tl::enums::InputBotInlineResult>,
) -> tl::functions::messages::SetInlineBotResults {
    tl::functions::messages::SetInlineBotResults {
        gallery: false,
        private: true,
        query_id,
        results,
        cache_time: 15,
        // Inline lookup returns one report. Sales pagination lives inside that report.
        // Omit the optional offset and its TL flag when there is no next result page.
        next_offset: None,
        switch_pm: None,
        switch_webview: None,
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
        message: if reply.rich_html.is_some() {
            String::new()
        } else {
            reply.text.clone()
        },
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
        rich_message: super::interactions::rich(reply),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grammers_client::tl::Serializable;
    #[test]
    fn edit_wire_flags_select_prepared_rich_content_or_plain_fallback() {
        for html in [None, Some("<p>Estimate</p>".into())] {
            let reply = Reply {
                text: "Estimate: 12 GRAM".into(),
                rich_html: html,
                ..Default::default()
            };
            for prepared in [
                None,
                Some(
                    tl::types::InputRichMessageHtml {
                        rtl: false,
                        noautolink: true,
                        html: "<p>Estimate</p>".into(),
                        files: None,
                    }
                    .into(),
                ),
            ] {
                let is_rich = prepared.is_some();
                let message = crate::tests::message(42, "");
                let chat = chat_edit_request(&message, &reply, prepared.clone(), None);
                let inline = inline_edit_request(
                    tl::types::InputBotInlineMessageId {
                        dc_id: 4,
                        id: 42,
                        access_hash: 0,
                    }
                    .into(),
                    &reply,
                    prepared,
                    None,
                );
                for (text, rich, wire) in [
                    (&chat.message, &chat.rich_message, chat.to_bytes()),
                    (&inline.message, &inline.rich_message, inline.to_bytes()),
                ] {
                    assert_eq!(text.as_ref(), (!is_rich).then_some(&reply.text));
                    assert_eq!(rich.is_some(), is_rich);
                    let flags = u32::from_le_bytes(wire[4..8].try_into().unwrap());
                    assert_eq!(
                        flags & ((1 << 11) | (1 << 23)),
                        if is_rich { 1 << 23 } else { 1 << 11 }
                    );
                }
            }
        }
    }
}
