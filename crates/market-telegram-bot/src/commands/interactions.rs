//! Parse entry points into common commands; callback tokens retain only bounded public report state.
use super::views::View;
use super::*;
use market_telegram::{CallbackTarget, Event};
use std::{
    collections::{VecDeque, hash_map::RandomState},
    hash::BuildHasher,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Default)]
pub(super) struct Views {
    entries: Mutex<VecDeque<(u64, Instant, View)>>,
    next: AtomicU64,
    random: RandomState,
}
impl Views {
    pub fn insert(&self, view: View) -> Vec<u8> {
        let id = self
            .random
            .hash_one(self.next.fetch_add(1, Ordering::Relaxed));
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        entries.retain(|(_, at, _)| at.elapsed() < Duration::from_secs(1800));
        while entries.len() >= 256 {
            entries.pop_front();
        }
        entries.push_back((id, Instant::now(), view));
        format!("v:{id:016x}").into_bytes()
    }
    pub fn get(&self, id: u64) -> Option<View> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        entries.retain(|(_, at, _)| at.elapsed() < Duration::from_secs(1800));
        let index = entries.iter().position(|(key, _, _)| *key == id)?;
        let entry = entries.remove(index)?;
        let view = entry.2.clone();
        entries.push_back(entry);
        Some(view)
    }
}
pub(super) fn parse_event(event: &Event, usernames: &[String]) -> Option<CommandRequest> {
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
            let command = if let Some(query) = super::portfolio::inline(query, *user_id) {
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
                    query.action = PortfolioAction::Navigate;
                }
                return Some(CommandRequest {
                    command: Command::Portfolio(query),
                    user_id: *user_id,
                    message_id: 0,
                });
            }
            let token = text.strip_prefix("v:")?;
            if token.len() != 16 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            (
                Command::View(u64::from_str_radix(token, 16).ok()?),
                *user_id,
                0,
            )
        }
    };
    (user_id > 0).then_some(CommandRequest {
        command,
        user_id,
        message_id,
    })
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
    fn callbacks_expire_even_when_recently_accessed() {
        let views = Views::default();
        let token = views.insert(super::super::tests::report_view());
        let id = u64::from_str_radix(std::str::from_utf8(&token[2..]).unwrap(), 16).unwrap();
        let created = views.entries.lock().unwrap()[0].1;
        assert!(views.get(id).is_some());
        assert_eq!(views.entries.lock().unwrap()[0].1, created);
        views.entries.lock().unwrap()[0].1 = Instant::now() - Duration::from_secs(1801);
        assert!(views.get(id).is_none());
        assert!(views.entries.lock().unwrap().is_empty());
    }
}
