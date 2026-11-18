use super::{Command, CommandRequest, FloorQuery};
use market_gifts::{
    CurrencyId,
    valuation::{ModelLookup, StrategyId},
};
use market_telegram::IncomingMessage;

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

pub(crate) fn parse<S: AsRef<str>>(
    message: IncomingMessage<'_>,
    usernames: &[S],
) -> Option<CommandRequest> {
    if !message.private || message.outgoing {
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
        CommandKind::Portfolio => portfolio(arguments).unwrap_or(Command::Invalid(kind)),
        CommandKind::Floor => floor(arguments)
            .map(Command::Floor)
            .unwrap_or(Command::Invalid(kind)),
    };
    Some(CommandRequest {
        command,
        user_id: message.user_id?,
        message_id: message.message_id,
    })
}

// Command shorthand belongs here; catalog IDs and availability remain domain/actor concerns.
fn quote_currency(text: &str) -> Option<CurrencyId> {
    if text.len() > 128 || text.split_whitespace().count() > 1 {
        return None;
    }
    let symbol = text.trim().to_ascii_uppercase();
    CurrencyId::new(match symbol.as_str() {
        "" | "TON" => "GRAM".to_owned(),
        _ => symbol,
    })
    .ok()
}
fn floor(arguments: &str) -> Option<FloorQuery> {
    if arguments.len() > 1024 {
        return None;
    }
    let mut parts = arguments.split('|').map(str::trim);
    let model = ModelLookup::named(parts.next()?, parts.next()?).ok()?;
    let currency = quote_currency(parts.next().unwrap_or(""))?;
    let strategy = StrategyId::new(parts.next().unwrap_or("model_floor")).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(FloorQuery {
        model,
        currency,
        strategy,
    })
}

fn portfolio(arguments: &str) -> Option<Command> {
    if arguments.len() > 256 {
        return None;
    }
    let mut parts = arguments.split('|').map(str::trim);
    let currency = quote_currency(parts.next().unwrap_or(""))?;
    let strategy = StrategyId::new(parts.next().unwrap_or("model_floor")).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(Command::Portfolio { currency, strategy })
}
