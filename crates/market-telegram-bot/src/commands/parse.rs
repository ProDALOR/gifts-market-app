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
    Multiplier,
}
impl CommandKind {
    pub const ALL: [Self; 7] = [
        Self::Start,
        Self::Market,
        Self::Help,
        Self::Status,
        Self::Floor,
        Self::Portfolio,
        Self::Multiplier,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Market => "market",
            Self::Help => "help",
            Self::Status => "status",
            Self::Floor => "floor",
            Self::Portfolio => "portfolio",
            Self::Multiplier => "multiplier",
        }
    }
    pub const fn description(self) -> &'static str {
        match self {
            Self::Start => "Open gift markets",
            Self::Market => "Browse markets",
            Self::Help => "How to use the app",
            Self::Status => "Check market data status",
            Self::Floor => "Gift floor with an explicit strategy",
            Self::Portfolio => "Value your visible gifts with an explicit strategy",
            Self::Multiplier => "Measure a model or backdrop ask-floor ratio",
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
        CommandKind::Multiplier => multiplier(arguments)
            .map(Command::Multiplier)
            .unwrap_or(Command::Invalid(kind)),
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
    let strategy = StrategyId::new(parts.next()?).ok()?;
    if !strategy.is_portfolio_strategy() {
        return None;
    }
    let model = ModelLookup::named(parts.next()?, parts.next()?).ok()?;
    let currency = quote_currency(parts.next().unwrap_or(""))?;
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
    let strategy = StrategyId::new(parts.next()?).ok()?;
    if !strategy.is_portfolio_strategy() {
        return None;
    }
    let currency = quote_currency(parts.next().unwrap_or(""))?;
    if parts.next().is_some() {
        return None;
    }
    Some(Command::Portfolio { currency, strategy })
}

fn multiplier(arguments: &str) -> Option<market_gifts::valuation::ValuationQuery> {
    use market_gifts::valuation::*;
    if arguments.len() > 512 {
        return None;
    }
    let mut parts = arguments.split('|').map(str::trim);
    let strategy = parts.next()?;
    let collection_id = market_gifts::CollectionId::new(parts.next()?).ok()?;
    let trait_id = parts.next()?;
    let currency_id = quote_currency(parts.next().unwrap_or(""))?;
    if parts.next().is_some() {
        return None;
    }
    match strategy {
        "model_multiplier" => Some(ValuationQuery::ModelMultiplier(ModelMultiplierRequest {
            collection_id,
            model_id: market_gifts::ModelId::new(trait_id).ok()?,
            currency_id,
        })),
        "backdrop_multiplier" => Some(ValuationQuery::BackdropMultiplier(
            BackdropMultiplierRequest {
                collection_id,
                backdrop_id: market_gifts::BackdropId::new(trait_id).ok()?,
                currency_id,
            },
        )),
        _ => None,
    }
}
