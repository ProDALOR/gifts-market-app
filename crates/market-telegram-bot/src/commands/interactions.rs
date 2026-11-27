//! Parse message, inline and guest entry points into typed application requests.
use super::*;
use market_telegram::{CallbackTarget, Event};
pub(super) fn parse_event(event: &Event, usernames: &[String]) -> Option<CommandRequest> {
    let inline = matches!(
        event,
        Event::Inline { .. }
            | Event::Guest { .. }
            | Event::Callback {
                target: CallbackTarget::Inline(_),
                ..
            }
    );
    let (command, user_id, message_id) = match event {
        Event::Message(message) => {
            return super::parse::parse(market_telegram::transport::incoming(message), usernames);
        }
        Event::Inline {
            user_id,
            query,
            offset,
            ..
        } => {
            if !offset.is_empty() {
                return None;
            }
            let command = if query.starts_with("s:") {
                Command::Help
            } else if let Some(command) = sharing::parse_target(query) {
                command
            } else if let Some(query) = super::portfolio::inline(query, *user_id) {
                Command::Portfolio(query)
            } else {
                Command::Gift(super::parse::gift(query, &[])?)
            };
            (command, *user_id, 0)
        }
        Event::Guest {
            user_id,
            text,
            references,
            ..
        } => {
            let input = super::parse::gift(text, references);
            (
                input.map(Command::Gift).unwrap_or(Command::GiftHelp),
                *user_id,
                0,
            )
        }
        Event::Callback {
            user_id,
            data,
            target,
            ..
        } => {
            let text = std::str::from_utf8(data).ok()?;
            if let Some(mut query) = super::portfolio::callback(text) {
                if matches!(target, CallbackTarget::Inline(_)) {
                    query.mode = PortfolioMode::Inline;
                    if query.action != PortfolioAction::Preview {
                        query.action = PortfolioAction::Navigate;
                    }
                }
                return Some(CommandRequest {
                    command: Command::Portfolio(query),
                    user_id: *user_id,
                    message_id: 0,
                    inline,
                });
            }
            (Command::Report(text.parse().ok()?), *user_id, 0)
        }
    };
    (user_id > 0).then_some(CommandRequest {
        command,
        user_id,
        message_id,
        inline,
    })
}

pub(super) fn callback_id(token: &str) -> Option<u64> {
    (token.len() == 16 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u64::from_str_radix(token, 16).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_portfolios_use_cached_subjects_and_reject_more_result_pages() {
        for query in [
            "",
            "portfolio",
            "portfolio sort=price_desc",
            "/portfolio 123 XTR sort=number_desc",
        ] {
            let event = Event::Inline {
                id: 1,
                user_id: 7,
                query: query.into(),
                offset: String::new(),
            };
            let request = parse_event(&event, &[]).unwrap();
            let Command::Portfolio(portfolio) = request.command else {
                panic!("portfolio query")
            };
            assert_eq!(portfolio.mode, PortfolioMode::Inline);
            assert_eq!(portfolio.action, PortfolioAction::Open);
            assert_eq!(request.user_id, 7);
        }
        let event = Event::Inline {
            id: 1,
            user_id: 7,
            query: "portfolio".into(),
            offset: "next".into(),
        };
        assert!(parse_event(&event, &[]).is_none());
        let gift = Event::Inline {
            id: 2,
            user_id: 7,
            query: "SakuraFlower-59197".into(),
            offset: String::new(),
        };
        assert!(matches!(
            parse_event(&gift, &[]).unwrap().command,
            Command::Gift(_)
        ));
        let callback = Event::Callback {
            id: 3,
            user_id: 99,
            data: b"p:123:GRAM:r:1:vd".to_vec(),
            target: CallbackTarget::Inline(market_telegram::InlineMessageId::Standard {
                dc_id: 2,
                id: 1,
                access_hash: 0,
            }),
        };
        let Command::Portfolio(shared) = parse_event(&callback, &[]).unwrap().command else {
            panic!("portfolio callback")
        };
        assert_eq!(
            (
                shared.user_id.get(),
                shared.mode,
                shared.sort,
                shared.action
            ),
            (
                123,
                PortfolioMode::Inline,
                market_portfolio::PortfolioSort::ValueDesc,
                PortfolioAction::Navigate
            )
        );
    }
    #[test]
    fn inline_gallery_preview_callbacks_remain_explicit_cached_previews() {
        let event = Event::Callback {
            id: 1,
            user_id: 7,
            data: b"p:123:GRAM:g:3:pa:g".to_vec(),
            target: CallbackTarget::Inline(market_telegram::InlineMessageId::Standard {
                dc_id: 2,
                id: 1,
                access_hash: 0,
            }),
        };
        let Command::Portfolio(query) = parse_event(&event, &[]).unwrap().command else {
            panic!()
        };
        assert_eq!(query.action, PortfolioAction::Preview);
        assert_eq!(query.layout, GridMode::Gallery);
        assert_eq!(query.mode, PortfolioMode::Inline);
    }
    #[test]
    fn retired_snapshot_queries_offer_help_without_resolving_a_cache_token() {
        let event = Event::Inline {
            id: 1,
            user_id: 7,
            query: "s:0123456789abcdef".into(),
            offset: String::new(),
        };
        assert_eq!(parse_event(&event, &[]).unwrap().command, Command::Help);
    }
}
