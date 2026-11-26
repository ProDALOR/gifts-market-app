use super::ValueRequest;
use super::{Command, CommandRequest};
use market_gifts::GiftSlug;
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
        self.details().0
    }
    pub const fn description(self) -> &'static str {
        self.details().1
    }
    pub(super) const fn usage(self) -> &'static str {
        self.details().2
    }
    const fn details(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Portfolio => (
                "portfolio",
                "Visible gift portfolio",
                "/portfolio [user_id] [GRAM|XTR] [sort=price_desc]",
            ),
            Self::Start => ("start", "Open gift markets", "/start"),
            Self::Market => (
                "market",
                "Browse markets",
                "/market [collection=ResistanceDog model=Dogfather backdrop=Black symbol=Crown]",
            ),
            Self::Help => ("help", "How to use the app", "/help"),
            Self::Status => ("status", "Check market data status", "/status"),
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
                inline: false,
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
            let args = text
                .split_once(char::is_whitespace)
                .map_or("", |(_, args)| args);
            Command::Portfolio(super::portfolio::parse_args(
                args,
                message.user_id?,
                super::PortfolioMode::Private,
            )?)
        }
        CommandKind::Start => Command::Start,
        CommandKind::Market => {
            if text.split_whitespace().count() > 1 {
                Command::Browse(super::browse::parse(text)?)
            } else {
                Command::Start
            }
        }
        CommandKind::Help => Command::Help,
        CommandKind::Status => Command::Status,
    };
    Some(CommandRequest {
        command,
        user_id: message.user_id?,
        message_id: message.message_id,
        inline: false,
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
        let Some(next) = super::BotCurrency::parse(word) else {
            continue;
        };
        if currency.is_some_and(|currency| currency != next) {
            return None;
        }
        currency = Some(next);
    }
    Some(ValueRequest {
        gift: slug.clone(),
        currency_id: currency.unwrap_or_default().id(),
    })
}
