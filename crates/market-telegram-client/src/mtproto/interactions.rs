//! Rich payloads and route-specific responses share the ordinary invocation scheduler.
use super::{client::MtProtoActions, invocations::Group, requests::reply_request};
use crate::{
    BotError, LaunchButton, Reply, ReplyButton,
    interactions::{CallbackTarget, Event, InlineMessageId},
};
use grammers_client::{
    message::{Button, ReplyMarkup},
    tl,
};
use market_invocations::{Priority, Replay};

pub(super) fn rich(reply: &Reply) -> Option<tl::enums::InputRichMessage> {
    reply.rich_html.as_ref().map(|html| {
        tl::types::InputRichMessageHtml {
            rtl: false,
            noautolink: true,
            html: html.clone(),
            files: None,
        }
        .into()
    })
}
pub(super) fn markup(
    reply: &Reply,
    username: Option<&str>,
    launch: Option<&LaunchButton>,
    inline_enabled: bool,
) -> Option<tl::enums::ReplyMarkup> {
    let rows: Vec<Vec<Button>> = reply
        .buttons
        .iter()
        .map(|row| {
            row.iter()
                .filter_map(|button| match button {
                    ReplyButton::Url { text, url } => Some(Button::url(text, url.as_str())),
                    ReplyButton::Callback { text, data } => Some(Button::data(text, data.clone())),
                    ReplyButton::SwitchInline { text, query } => {
                        inline_enabled.then(|| Button::switch_elsewhere(text, query))
                    }
                    ReplyButton::Preview { text, payload } => launch.map(|launch| {
                        let url = if let Some(name) = username {
                            format!("https://t.me/{name}?startapp={payload}")
                        } else {
                            let mut url = launch.url.clone();
                            url.query_pairs_mut()
                                .append_pair("tgWebAppStartParam", payload);
                            url.to_string()
                        };
                        Button::url(text, url)
                    }),
                })
                .collect()
        })
        .filter(|row: &Vec<_>| !row.is_empty())
        .collect();
    if rows.is_empty() {
        None
    } else {
        Some(ReplyMarkup::from_buttons(&rows).raw)
    }
}
pub(super) fn inline_result(
    reply: &Reply,
    username: Option<&str>,
    launch: Option<&LaunchButton>,
    rich_message: Option<tl::enums::InputRichMessage>,
    inline_enabled: bool,
) -> tl::enums::InputBotInlineResult {
    let markup = markup(reply, username, launch, inline_enabled);
    let send_message = if let Some(rich_message) = rich_message {
        tl::types::InputBotInlineMessageRichMessage {
            reply_markup: markup,
            rich_message,
        }
        .into()
    } else {
        tl::types::InputBotInlineMessageText {
            no_webpage: true,
            invert_media: false,
            message: reply.text.clone(),
            entities: None,
            reply_markup: markup,
        }
        .into()
    };
    tl::types::InputBotInlineResult {
        id: "valuation".into(),
        r#type: "article".into(),
        title: Some(reply.title.clone().unwrap_or_else(|| {
            reply
                .text
                .lines()
                .next()
                .unwrap_or("Gift valuation")
                .chars()
                .take(100)
                .collect()
        })),
        description: Some(reply.summary.clone().unwrap_or_else(|| {
            reply
                .text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .skip(1)
                .take(2)
                .collect::<Vec<_>>()
                .join(" · ")
                .chars()
                .take(180)
                .collect()
        })),
        url: None,
        thumb: None,
        content: None,
        send_message,
    }
    .into()
}
impl From<&InlineMessageId> for tl::enums::InputBotInlineMessageId {
    fn from(id: &InlineMessageId) -> Self {
        match id {
            InlineMessageId::Standard {
                dc_id,
                id,
                access_hash,
            } => tl::types::InputBotInlineMessageId {
                dc_id: *dc_id,
                id: *id,
                access_hash: *access_hash,
            }
            .into(),
            InlineMessageId::Wide {
                dc_id,
                owner_id,
                id,
                access_hash,
            } => tl::types::InputBotInlineMessageId64 {
                dc_id: *dc_id,
                owner_id: *owner_id,
                id: *id,
                access_hash: *access_hash,
            }
            .into(),
        }
    }
}
impl From<&tl::enums::InputBotInlineMessageId> for InlineMessageId {
    fn from(id: &tl::enums::InputBotInlineMessageId) -> Self {
        match id {
            tl::enums::InputBotInlineMessageId::Id(value) => Self::Standard {
                dc_id: value.dc_id,
                id: value.id,
                access_hash: value.access_hash,
            },
            tl::enums::InputBotInlineMessageId::Id64(value) => Self::Wide {
                dc_id: value.dc_id,
                owner_id: value.owner_id,
                id: value.id,
                access_hash: value.access_hash,
            },
        }
    }
}
impl MtProtoActions {
    pub(super) async fn respond(
        &self,
        event: &Event,
        reply: Option<&Reply>,
        launch: Option<&LaunchButton>,
    ) -> Result<(), BotError> {
        let username = self
            .profile
            .get()
            .filter(|user| user.has_main_web_app)
            .map(|user| user.username.as_str());
        let inline_enabled = self.profile.get().is_some_and(|user| user.inline_enabled);
        let prepared = self.prepare_rich(reply).await;
        match event {
            Event::Message(message) => {
                let Some(reply) = reply else { return Ok(()) };
                let mut request = reply_request(self.credentials.bot_id(), message, reply, launch);
                request.reply_markup =
                    markup(reply, username, launch, inline_enabled).or(request.reply_markup);
                request.rich_message = prepared;
                self.invocations
                    .rpc(
                        &self.client,
                        request,
                        Group::Reply(message.peer.id.bare_id().ok_or(BotError::InvalidResponse)?),
                        Priority::Interactive,
                        Replay::Deduplicated,
                    )
                    .await?;
            }
            Event::Inline { id, .. } => {
                self.invocations
                    .rpc(
                        &self.client,
                        tl::functions::messages::SetInlineBotResults {
                            gallery: false,
                            private: true,
                            query_id: *id,
                            results: reply
                                .map(|reply| {
                                    vec![inline_result(
                                        reply,
                                        username,
                                        launch,
                                        prepared.clone(),
                                        inline_enabled,
                                    )]
                                })
                                .unwrap_or_default(),
                            cache_time: 15,
                            next_offset: Some(String::new()),
                            switch_pm: None,
                            switch_webview: None,
                        },
                        Group::Inline,
                        Priority::Interactive,
                        Replay::Safe,
                    )
                    .await?;
            }
            Event::Guest { id, .. } => {
                let Some(reply) = reply else { return Ok(()) };
                // A guest query grants one response; retrying an ambiguous write is unsafe.
                self.invocations
                    .rpc(
                        &self.client,
                        tl::functions::messages::SetBotGuestChatResult {
                            query_id: *id,
                            result: inline_result(
                                reply,
                                username,
                                launch,
                                prepared.clone(),
                                inline_enabled,
                            ),
                        },
                        Group::Inline,
                        Priority::Interactive,
                        Replay::Never,
                    )
                    .await?;
            }
            Event::Callback { target, .. } => {
                let Some(reply) = reply else { return Ok(()) };
                let markup = markup(reply, username, launch, inline_enabled);
                match target {
                    CallbackTarget::Message(message) => {
                        self.invocations
                            .rpc(
                                &self.client,
                                tl::functions::messages::EditMessage {
                                    no_webpage: true,
                                    invert_media: false,
                                    peer: message.peer.into(),
                                    id: message.id,
                                    message: Some(if reply.rich_html.is_some() {
                                        String::new()
                                    } else {
                                        reply.text.clone()
                                    }),
                                    media: None,
                                    reply_markup: markup,
                                    entities: None,
                                    schedule_date: None,
                                    schedule_repeat_period: None,
                                    quick_reply_shortcut_id: None,
                                    rich_message: prepared,
                                },
                                Group::Reply(
                                    message.peer.id.bare_id().ok_or(BotError::InvalidResponse)?,
                                ),
                                Priority::Interactive,
                                Replay::Safe,
                            )
                            .await?;
                    }
                    CallbackTarget::Inline(id) => {
                        self.invocations
                            .rpc(
                                &self.client,
                                tl::functions::messages::EditInlineBotMessage {
                                    no_webpage: true,
                                    invert_media: false,
                                    id: id.into(),
                                    message: Some(if reply.rich_html.is_some() {
                                        String::new()
                                    } else {
                                        reply.text.clone()
                                    }),
                                    media: None,
                                    reply_markup: markup,
                                    entities: None,
                                    rich_message: prepared,
                                },
                                Group::Inline,
                                Priority::Interactive,
                                Replay::Safe,
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rich_inline_and_guest_payloads_share_html_and_use_url_preview_buttons() {
        let reply = Reply {
            text: "Gift-123\nEstimate: 18 TON".into(),
            rich_html: Some("<h3>Gift-123</h3><p><tg-emoji emoji-id=\"123\">💎</tg-emoji> <b>18 TON</b></p><details><summary>🕘 Recent sales</summary><p>Observed executions</p></details>".into()),
            buttons: vec![vec![ReplyButton::Preview {
                text: "Open preview".into(),
                payload: "v_Z2lmdD1HaWZ0LTEyMw".into(),
            }]],
            ..Default::default()
        }
        .bounded();
        let launch = LaunchButton {
            text: "Market".into(),
            url: url::Url::parse("https://market.example").unwrap(),
        };
        let tl::enums::InputBotInlineResult::Result(result) = inline_result(
            &reply,
            Some("market_bot"),
            Some(&launch),
            rich(&reply),
            true,
        ) else {
            panic!()
        };
        assert_eq!(result.r#type, "article");
        let tl::enums::InputBotInlineMessage::RichMessage(message) = result.send_message else {
            panic!()
        };
        let tl::enums::InputRichMessage::Html(html) = message.rich_message else {
            panic!()
        };
        assert_eq!(html.html, reply.rich_html.unwrap());
        assert!(message.reply_markup.is_some());
        let plain = Reply::text("Help");
        assert!(
            markup(&plain, Some("market_bot"), Some(&launch), true).is_none(),
            "inline/guest responses cannot use private-only WebView buttons"
        );
    }
    #[test]
    fn both_inline_message_identity_encodings_round_trip_without_losing_owner_or_dc() {
        for id in [
            InlineMessageId::Standard {
                dc_id: 2,
                id: i64::MAX,
                access_hash: 123,
            },
            InlineMessageId::Wide {
                dc_id: 4,
                owner_id: i64::MAX,
                id: 42,
                access_hash: -123,
            },
        ] {
            assert_eq!(
                InlineMessageId::from(&tl::enums::InputBotInlineMessageId::from(&id)),
                id
            );
        }
    }
}
