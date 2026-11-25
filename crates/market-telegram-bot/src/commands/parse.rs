use super::ValueRequest;
use super::{Command, CommandRequest};
use market_gifts::{CurrencyId, GiftSlug};
use market_telegram::IncomingMessage;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Start,
    Market,
    Help,
    Status,
    Portfolio,
}
impl CommandKind {
    pub const ALL: [Self; 5] = [
        Self::Start,
        Self::Market,
        Self::Help,
        Self::Status,
        Self::Portfolio,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Portfolio => "portfolio",
            Self::Start => "start",
            Self::Market => "market",
            Self::Help => "help",
            Self::Status => "status",
        }
    }
    pub const fn description(self) -> &'static str {
        match self {
            Self::Portfolio => "Visible gift portfolio",
            Self::Start => "Open gift markets",
            Self::Market => "Browse markets",
            Self::Help => "How to use the app",
            Self::Status => "Check market data status",
        }
    }
}
pub(crate) fn parse<S: AsRef<str>>(
    message: IncomingMessage<'_>,
    usernames: &[S],
) -> Option<CommandRequest> {
    if !message.private || message.outgoing {
        return None;
    }
    let text = message.text?.trim();
    if !text.starts_with('/') {
        return gift(text, &[])
            .map(|query| CommandRequest {
                command: Command::Gift(query),
                user_id: message.user_id.unwrap_or(0),
                message_id: message.message_id,
            })
            .filter(|request| request.user_id > 0);
    }
    let name = text.split_whitespace().next()?;
    let (name, target) = name
        .split_once('@')
        .map_or((name, None), |(name, target)| (name, Some(target)));
    if target.is_some_and(|target| {
        !usernames
            .iter()
            .any(|name| target.eq_ignore_ascii_case(name.as_ref()))
    }) {
        return None;
    }
    let kind = CommandKind::ALL
        .into_iter()
        .find(|kind| Some(kind.name()) == name.strip_prefix('/'))?;
    let command = match kind {
        CommandKind::Portfolio => {
            let mut words = text.split_whitespace().skip(1);
            let user_id = words
                .next()
                .map(|v| v.parse::<u64>().ok())
                .unwrap_or(message.user_id)?;
            market_portfolio::UserId::new(user_id).ok()?;
            let currency_id = words.next().unwrap_or("GRAM").to_owned().try_into().ok()?;
            if words.next().is_some() {
                return None;
            }
            Command::Portfolio(super::PortfolioRequest {
                user_id,
                currency_id,
                refresh: false,
                page: 0,
            })
        }
        CommandKind::Start => Command::Start,
        CommandKind::Market => {
            if text.split_whitespace().count() > 1 {
                Command::Browse(super::browse::parse(text)?)
            } else {
                Command::Market
            }
        }
        CommandKind::Help => Command::Help,
        CommandKind::Status => Command::Status,
    };
    Some(CommandRequest {
        command,
        user_id: message.user_id?,
        message_id: message.message_id,
    })
}

/// Find one gift without interpreting surrounding words as commands or identities.
/// Guest reply context is consulted only when the summoning message has no gift.
pub(super) fn gift(text: &str, references: &[String]) -> Option<ValueRequest> {
    let text = text.trim();
    if text.len() > 2048 || text.starts_with('/') {
        return None;
    }
    let candidates = |text: &str| {
        text.split_whitespace()
            .take(128)
            .filter_map(|word| {
                GiftSlug::new(word.trim_matches(|c: char| {
                    matches!(
                        c,
                        '<' | '>'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | ','
                            | ';'
                            | '.'
                            | ':'
                            | '?'
                            | '!'
                            | '"'
                            | '\''
                    )
                }))
                .ok()
            })
            .collect::<Vec<_>>()
    };
    let mut found = candidates(text);
    if found.is_empty() {
        for reference in references.iter().take(4) {
            found.extend(candidates(reference));
        }
    }
    let slug = found.first()?;
    if found.iter().any(|value| value != slug) {
        return None;
    }
    let mut currency = None;
    for word in text.split_whitespace() {
        let word =
            word.trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '?' | '!' | '(' | ')'));
        let next = match word.to_ascii_uppercase().as_str() {
            "GRAM" | "TON" => "GRAM",
            "XTR" => "XTR",
            _ => continue,
        };
        if currency.is_some_and(|currency| currency != next) {
            return None;
        }
        currency = Some(next);
    }
    Some(ValueRequest {
        gift: slug.clone(),
        currency_id: CurrencyId::from(currency.unwrap_or("GRAM")),
    })
}
