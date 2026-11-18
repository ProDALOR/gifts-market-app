use super::{Command, CommandRequest};
use market_gifts::{CurrencyId, GiftSlug, valuation::GiftValuationRequest};
use market_telegram::IncomingMessage;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Start,
    Market,
    Help,
    Status,
    Value,
}
impl CommandKind {
    pub const ALL: [Self; 5] = [
        Self::Start,
        Self::Market,
        Self::Help,
        Self::Status,
        Self::Value,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Market => "market",
            Self::Help => "help",
            Self::Status => "status",
            Self::Value => "value",
        }
    }
    pub const fn description(self) -> &'static str {
        match self {
            Self::Start => "Open gift markets",
            Self::Market => "Browse markets",
            Self::Help => "How to use the app",
            Self::Status => "Check market data status",
            Self::Value => "Value a gift URL or slug with all strategies",
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
        return value(text)
            .map(|query| CommandRequest {
                command: Command::Value(query),
                user_id: message.user_id.unwrap_or(0),
                message_id: message.message_id,
            })
            .filter(|request| request.user_id > 0);
    }
    let (name, arguments) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
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
    let arguments = arguments.trim();
    let command = match kind {
        CommandKind::Start => Command::Start,
        CommandKind::Market => Command::Market,
        CommandKind::Help => Command::Help,
        CommandKind::Status => Command::Status,
        CommandKind::Value => value(arguments)
            .map(Command::Value)
            .unwrap_or(Command::Invalid(kind)),
    };
    Some(CommandRequest {
        command,
        user_id: message.user_id?,
        message_id: message.message_id,
    })
}

fn value(arguments: &str) -> Option<GiftValuationRequest> {
    if arguments.len() > 2048 {
        return None;
    }
    let mut parts = arguments.split_whitespace();
    let gift = GiftSlug::new(parts.next()?).ok()?;
    let currency = parts.next().unwrap_or("GRAM").to_ascii_uppercase();
    let currency_id = CurrencyId::new(if currency == "TON" { "GRAM" } else { &currency }).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(GiftValuationRequest {
        gift: gift.into(),
        currency_id,
    })
}
