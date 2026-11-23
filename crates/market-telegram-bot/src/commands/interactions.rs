//! Parse entry points into common commands; callback tokens retain only bounded public report state.
use super::*;
use super::{history::HistoryRequest, report::Report};
use market_telegram::Event;
use std::{
    collections::{VecDeque, hash_map::RandomState},
    hash::BuildHasher,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone)]
pub(super) struct View {
    pub report: Arc<Report>,
    pub history: HistoryRequest,
}
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
            (Command::Gift(super::parse::gift(query, &[])?), *user_id, 0)
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
        Event::Callback { user_id, data, .. } => {
            let text = std::str::from_utf8(data).ok()?;
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

impl MarketCommands {
    pub(super) async fn view(&self, id: u64) -> Reply {
        match self.views.get(id) {
            Some(view) => self.history(view.report, view.history).await,
            None => Reply::text(
                "This report has expired. Send the gift link again for a fresh valuation.",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callbacks_expire_even_when_recently_accessed() {
        let views = Views::default();
        let token = views.insert(super::super::tests::history_view());
        let id = u64::from_str_radix(std::str::from_utf8(&token[2..]).unwrap(), 16).unwrap();
        let created = views.entries.lock().unwrap()[0].1;
        assert!(views.get(id).is_some());
        assert_eq!(views.entries.lock().unwrap()[0].1, created);
        views.entries.lock().unwrap()[0].1 = Instant::now() - Duration::from_secs(1801);
        assert!(views.get(id).is_none());
        assert!(views.entries.lock().unwrap().is_empty());
    }
}
