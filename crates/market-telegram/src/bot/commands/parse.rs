use super::{Command, CommandRequest, FloorQuery, QuoteCurrency};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Start,
    Market,
    Help,
    Status,
    Floor,
    Portfolio,
}
impl CommandKind {
    pub const ALL: [Self; 6] = [
        Self::Start,
        Self::Market,
        Self::Help,
        Self::Status,
        Self::Floor,
        Self::Portfolio,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Market => "market",
            Self::Help => "help",
            Self::Status => "status",
            Self::Floor => "floor",
            Self::Portfolio => "portfolio",
        }
    }
    pub const fn description(self) -> &'static str {
        match self {
            Self::Start => "Open gift markets",
            Self::Market => "Browse markets",
            Self::Help => "How to use the app",
            Self::Status => "Check market data status",
            Self::Floor => "Model floor: collection | model",
            Self::Portfolio => "Value your visible gifts at model floors",
        }
    }
}

pub(in crate::bot) struct Incoming<'a> {
    pub private: bool,
    pub user_id: Option<u64>,
    pub message_id: i32,
    pub text: Option<&'a str>,
}
pub(in crate::bot) fn parse<S: AsRef<str>>(
    message: Incoming<'_>,
    usernames: &[S],
) -> Option<CommandRequest> {
    if !message.private {
        return None;
    }
    let text = message.text?.trim();
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
        CommandKind::Portfolio => QuoteCurrency::parse(arguments)
            .map(Command::Portfolio)
            .unwrap_or(Command::Invalid(kind)),
        CommandKind::Floor => {
            let parts: Vec<_> = arguments.split('|').map(str::trim).collect();
            if arguments.len() > 1024
                || !(2..=3).contains(&parts.len())
                || parts[..2]
                    .iter()
                    .any(|part| part.is_empty() || part.len() > 256)
            {
                Command::Invalid(kind)
            } else if let Some(currency) = QuoteCurrency::parse(parts.get(2).copied().unwrap_or(""))
            {
                Command::Floor(FloorQuery {
                    collection: parts[0].into(),
                    model: parts[1].into(),
                    currency,
                })
            } else {
                Command::Invalid(kind)
            }
        }
    };
    Some(CommandRequest {
        command,
        user_id: message.user_id?,
        message_id: message.message_id,
    })
}
