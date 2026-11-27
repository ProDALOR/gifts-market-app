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
    pub const PUBLIC: [Self; 2] = [Self::Start, Self::Help];
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
    const fn details(self) -> (&'static str, &'static str) {
        match self {
            Self::Portfolio => ("portfolio", "Visible gift portfolio"),
            Self::Start => ("start", "Open gift markets"),
            Self::Market => ("market", "Browse markets"),
            Self::Help => ("help", "How to use the app"),
            Self::Status => ("status", "Check market data status"),
        }
    }
}
pub(crate) fn parse<S: AsRef<str>>(
    message: IncomingMessage<'_>,
    usernames: &[S],
) -> Option<CommandRequest> {
    parse_with_origin(message, usernames, None)
}
pub(super) fn parse_with_origin<S: AsRef<str>>(
    message: IncomingMessage<'_>,
    usernames: &[S],
    origin: Option<&url::Url>,
) -> Option<CommandRequest> {
    if !message.private || message.outgoing {
        return None;
    }
    let text = message.text?.trim();
    if text.is_empty() {
        return None;
    }
    if !text.starts_with('/') {
        return input(text, message.user_id?, usernames, origin, &[])
            .map(|command| CommandRequest {
                command,
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
        CommandKind::Start => {
            let args = text
                .split_once(char::is_whitespace)
                .map(|(_, args)| args.trim())
                .unwrap_or("");
            if args.is_empty() {
                Command::Start
            } else {
                market_links::Target::from_launch(args)
                    .ok()
                    .filter(|_| args.len() <= 64)
                    .and_then(|target| command_target(target).ok())
                    .unwrap_or(Command::Help)
            }
        }
        CommandKind::Market => {
            if text.split_whitespace().count() > 1 {
                Command::Browse(super::browse::parse(&format!(
                    "/market {}",
                    text.split_once(char::is_whitespace)?.1
                ))?)
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

/// Interpret a resolved navigation subject without fetching inventory.
pub(super) fn command_target(target: market_links::Target) -> Result<Command, ()> {
    use market_links::Target;
    match target {
        Target::Gift { slug, currency_id } => {
            let currency = super::BotCurrency::parse(currency_id.as_str()).ok_or(())?;
            Ok(Command::Gift(ValueRequest {
                gift: slug,
                currency_id: currency.id(),
            }))
        }
        Target::Market { selection } => {
            super::BotCurrency::parse(selection.filters.currency_id.as_str()).ok_or(())?;
            if let Some(slug) = selection.gift {
                return command_target(Target::Gift {
                    slug,
                    currency_id: selection.filters.currency_id,
                });
            }
            Ok(Command::Browse(selection))
        }
        Target::Portfolio { target } => {
            let currency = super::BotCurrency::parse(target.currency_id.as_str()).ok_or(())?;
            if target.filters == Default::default() {
                Ok(Command::Portfolio(super::PortfolioRequest {
                    user_id: target.user_id,
                    currency_id: currency,
                    action: super::PortfolioAction::Open,
                    page: 0,
                    sort: target.sort,
                    mode: super::PortfolioMode::Inline,
                    layout: super::GridMode::Linked,
                }))
            } else {
                Ok(Command::PortfolioTarget(target))
            }
        }
    }
}
pub(super) fn target<S: AsRef<str>>(
    text: &str,
    user: u64,
    usernames: &[S],
    origin: Option<&url::Url>,
) -> Option<Command> {
    let target = if direct_gift(text) {
        market_links::Target::parse_text(text, user).ok()?
    } else if let Ok(url) = url::Url::parse(text) {
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }
        if url.host_str() == Some("t.me") && !url.path().starts_with("/nft/") {
            if !usernames.iter().any(|name| {
                url.path()
                    .trim_matches('/')
                    .eq_ignore_ascii_case(name.as_ref())
            }) {
                return None;
            }
            market_links::Target::from_bot_url(&url, user).ok()?
        } else if origin.is_some_and(|own| own.origin() == url.origin()) {
            market_links::Target::from_url(&url).ok()?
        } else {
            return None;
        }
    } else {
        market_links::Target::parse_text(text, user).ok()?
    };
    command_target(target).ok()
}

fn direct_gift(text: &str) -> bool {
    let mut words = text.split_whitespace();
    words.next().is_some_and(|word| GiftSlug::new(word).is_ok()) && words.count() <= 1
}
/// A malformed explicit target must never fall back to a different gift or currency.
pub(super) fn input<S: AsRef<str>>(
    text: &str,
    user: u64,
    usernames: &[S],
    origin: Option<&url::Url>,
    references: &[String],
) -> Option<Command> {
    if let Some(command) = target(text, user, usernames, origin) {
        return Some(command);
    }
    if !text.trim().is_empty()
        && let Some(request) = super::portfolio::inline(text, user)
    {
        return Some(Command::Portfolio(request));
    }
    let name = text.split_whitespace().next().unwrap_or("");
    if matches!(name, "market" | "/market" | "portfolio" | "/portfolio") || direct_gift(text) {
        return None;
    }
    gift(text, references).map(Command::Gift)
}
