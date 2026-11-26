//! Creator-bound controls for inline/guest reports, independent of cached report data.
use super::{
    Command, CommandRequest,
    interactions::{self, ViewCache},
};
use market_telegram::{CallbackTarget, Event, Reply, ReplyButton};

#[derive(Clone, PartialEq, Eq)]
struct Control {
    user: u64,
    data: Vec<u8>,
}
pub(super) struct InlineControls(ViewCache<Control>);
impl Default for InlineControls {
    fn default() -> Self {
        Self(ViewCache::new("i", 4096))
    }
}
impl InlineControls {
    fn id(data: &[u8]) -> Option<u64> {
        interactions::callback_id(std::str::from_utf8(data).ok()?.strip_prefix("i:")?)
    }
    fn bind(&self, user: u64, data: &[u8]) -> Vec<u8> {
        // Reusing a prepared/shared report creates independent controls for its new sender.
        let data = Self::id(data)
            .and_then(|id| self.0.get(id))
            .map_or_else(|| data.to_vec(), |control| control.data);
        self.0
            .find_or_insert(
                |control| control.user == user && control.data == data,
                || Control {
                    user,
                    data: data.clone(),
                },
            )
            .0
    }
    pub fn reply(&self, user: u64, mut reply: Reply) -> Reply {
        for button in reply.buttons.iter_mut().flatten() {
            if let ReplyButton::Callback { data, .. } = button {
                *data = self.bind(user, data);
            }
        }
        if let Some(html) = &mut reply.rich_html {
            // All callback markup is emitted by rich::button, with this exact escaped prefix.
            const PREFIX: &str = super::rich::CALLBACK_PREFIX;
            let mut remaining = html.as_str();
            let mut rendered = String::with_capacity(html.len());
            while let Some(start) = remaining.find(PREFIX) {
                let end_prefix = start + PREFIX.len();
                rendered.push_str(&remaining[..end_prefix]);
                remaining = &remaining[end_prefix..];
                let Some(end) = remaining.find('"') else {
                    break;
                };
                let token = self.bind(user, remaining[..end].as_bytes());
                rendered.push_str(std::str::from_utf8(&token).expect("ASCII control token"));
                remaining = &remaining[end..];
            }
            rendered.push_str(remaining);
            *html = rendered;
        }
        reply
    }
    pub fn parse(&self, event: &Event, usernames: &[String]) -> Option<CommandRequest> {
        let Event::Callback {
            id,
            user_id,
            data,
            target,
        } = event
        else {
            return interactions::parse_event(event, usernames);
        };
        let private_owner = match target {
            CallbackTarget::Message(message) => {
                market_telegram::transport::incoming(message).user_id == Some(*user_id)
            }
            CallbackTarget::Inline(_) => false,
        };
        if private_owner && !data.starts_with(b"i:") {
            return interactions::parse_event(event, usernames);
        }
        let control = Self::id(data).and_then(|id| self.0.get(id));
        let command = match control {
            Some(control) if control.user == *user_id => {
                let mut request = interactions::parse_event(
                    &Event::Callback {
                        id: *id,
                        user_id: *user_id,
                        data: control.data,
                        target: target.clone(),
                    },
                    usernames,
                )?;
                request.inline = true;
                return Some(request);
            }
            Some(_) => Command::CallbackDenied,
            None => Command::CallbackExpired,
        };
        Some(CommandRequest {
            command,
            user_id: *user_id,
            message_id: 0,
            inline: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_telegram::InlineMessageId;

    fn callback(user: u64, data: Vec<u8>) -> Event {
        Event::Callback {
            id: 1,
            user_id: user,
            data,
            // This Telegram routing field is not the report creator's identity.
            target: CallbackTarget::Inline(InlineMessageId::Wide {
                dc_id: 4,
                owner_id: 999,
                id: 5,
                access_hash: 6,
            }),
        }
    }
    fn token(reply: &Reply) -> Vec<u8> {
        let ReplyButton::Callback { data, .. } = &reply.buttons[0][0] else {
            panic!()
        };
        data.clone()
    }
    fn reply(data: &str) -> Reply {
        Reply {
            rich_html: Some(format!(
                "<p>Report</p>{}{}",
                super::super::rich::button("Next", Some(data), false),
                super::super::rich::button("Floors", None, true)
            )),
            buttons: vec![vec![
                ReplyButton::Callback {
                    text: "Next".into(),
                    data: data.as_bytes().to_vec(),
                },
                ReplyButton::SwitchInline {
                    text: "Share".into(),
                    query: "Gift-1".into(),
                },
                ReplyButton::Preview {
                    text: "Open".into(),
                    payload: "gift_1".into(),
                },
            ]],
            ..Reply::text("Report")
        }
    }
    #[test]
    fn rich_and_native_controls_are_creator_bound_and_share_no_mutable_reply() {
        let controls = InlineControls::default();
        for data in [
            "v:0000000000000001",
            "m:0000000000000002",
            "p:123:GRAM:v:1:vd",
        ] {
            let original = reply(data);
            let bound = controls.reply(7, original.clone());
            let protected = token(&bound);
            assert_eq!(protected.len(), 18);
            assert!(
                bound
                    .rich_html
                    .as_ref()
                    .unwrap()
                    .contains(std::str::from_utf8(&protected).unwrap())
            );
            assert!(!bound.rich_html.as_ref().unwrap().contains(data));
            assert_eq!(
                bound.buttons[0][1..],
                original.buttons[0][1..],
                "sharing and app links stay public"
            );
            assert_eq!(
                controls.reply(7, original.clone()),
                bound,
                "repeated destinations reuse tokens"
            );
            assert_eq!(
                controls.reply(7, bound.clone()),
                bound,
                "binding is idempotent"
            );
            let allowed = controls
                .parse(&callback(7, protected.clone()), &[])
                .unwrap();
            assert!(allowed.inline);
            assert!(matches!(
                allowed.command,
                Command::View(1) | Command::MarketView(2) | Command::Portfolio(_)
            ));
            assert_eq!(
                controls
                    .parse(&callback(8, protected), &[])
                    .unwrap()
                    .command,
                Command::CallbackDenied
            );
            assert_eq!(
                token(&original),
                data.as_bytes(),
                "cached source replies stay unmodified"
            );
        }
    }
    #[test]
    fn reshared_copies_bind_to_the_new_sender_and_old_raw_tokens_fail_closed() {
        let controls = InlineControls::default();
        let first = controls.reply(7, reply("v:0000000000000001"));
        let second = controls.reply(8, first.clone());
        assert_ne!(token(&first), token(&second));
        assert_eq!(
            controls
                .parse(&callback(7, token(&second)), &[])
                .unwrap()
                .command,
            Command::CallbackDenied
        );
        assert_eq!(
            controls
                .parse(&callback(8, token(&second)), &[])
                .unwrap()
                .command,
            Command::View(1)
        );
        assert_eq!(
            controls
                .parse(&callback(7, token(&first)), &[])
                .unwrap()
                .command,
            Command::View(1)
        );
        for data in [
            "v:0000000000000001",
            "m:0000000000000002",
            "p:123:GRAM:v:1:vd",
            "i:invalid",
            "i:0000000000000000",
        ] {
            assert_eq!(
                controls
                    .parse(&callback(7, data.as_bytes().to_vec()), &[])
                    .unwrap()
                    .command,
                Command::CallbackExpired
            );
        }
    }
    #[test]
    fn evicted_or_restarted_controls_fail_closed() {
        let controls = InlineControls(ViewCache::new("i", 1));
        let first = controls.reply(7, reply("v:0000000000000001"));
        let second = controls.reply(7, reply("v:0000000000000002"));
        assert_eq!(
            controls
                .parse(&callback(7, token(&first)), &[])
                .unwrap()
                .command,
            Command::CallbackExpired
        );
        assert_eq!(
            controls
                .parse(&callback(7, token(&second)), &[])
                .unwrap()
                .command,
            Command::View(2)
        );
        assert_eq!(
            InlineControls::default()
                .parse(&callback(7, token(&second)), &[])
                .unwrap()
                .command,
            Command::CallbackExpired
        );
    }

    #[test]
    fn private_callbacks_keep_working_and_group_callbacks_cannot_bypass_ownership() {
        use grammers_session::types::{PeerAuth, PeerId, PeerRef};
        let controls = InlineControls::default();
        let protected = controls.reply(7, reply("v:0000000000000001"));
        let message = |id| {
            CallbackTarget::Message(market_telegram::transport::Message {
                peer: PeerRef {
                    id,
                    auth: PeerAuth::from_hash(123),
                },
                id: 1,
                outgoing: true,
                text: String::new(),
            })
        };
        let event = |user_id, data, target| Event::Callback {
            id: 1,
            user_id,
            data,
            target,
        };
        let private = controls
            .parse(
                &event(
                    7,
                    b"v:0000000000000001".to_vec(),
                    message(PeerId::user(7).unwrap()),
                ),
                &[],
            )
            .unwrap();
        assert_eq!(private.command, Command::View(1));
        assert!(!private.inline);
        let group = message(PeerId::chat(9).unwrap());
        assert_eq!(
            controls
                .parse(
                    &event(7, b"v:0000000000000001".to_vec(), group.clone()),
                    &[]
                )
                .unwrap()
                .command,
            Command::CallbackExpired
        );
        assert_eq!(
            controls
                .parse(&event(8, token(&protected), group.clone()), &[])
                .unwrap()
                .command,
            Command::CallbackDenied
        );
        let creator = controls
            .parse(&event(7, token(&protected), group), &[])
            .unwrap();
        assert!(creator.inline);
        assert_eq!(creator.command, Command::View(1));
    }
}
