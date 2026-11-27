//! Lower public navigation targets to Telegram button intents.
use super::*;
pub type PortfolioTarget = market_links::PortfolioLink;
use market_links::ShareDestination;
#[derive(Debug)]
pub enum ShareError {
    Invalid(String),
    Unavailable(String),
}
impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) | Self::Unavailable(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for ShareError {}
pub(super) fn market_destination(
    selection: &MarketSelection,
    origin: Option<&url::Url>,
) -> Result<ShareDestination, ShareError> {
    market_links::Target::Market {
        selection: selection.clone(),
    }
    .share(origin)
    .map_err(|e| ShareError::Invalid(e.to_string()))
}
pub(super) fn portfolio_destination(
    target: &PortfolioTarget,
    origin: Option<&url::Url>,
) -> Result<ShareDestination, ShareError> {
    market_links::Target::Portfolio {
        target: target.clone(),
    }
    .share(origin)
    .map_err(|e| ShareError::Invalid(e.to_string()))
}
pub(super) fn with_share(mut reply: Reply, destination: &ShareDestination) -> Reply {
    for row in &mut reply.buttons {
        row.retain(|button| !matches!(button, market_telegram::ReplyButton::Share { .. }));
    }
    reply.buttons.retain(|row| !row.is_empty());
    let destination = match destination {
        ShareDestination::Inline { query } => market_telegram::ShareLink::Inline(query.clone()),
        ShareDestination::Link { url } => {
            let mut share = url::Url::parse("https://t.me/share/url").unwrap();
            share.query_pairs_mut().append_pair("url", url.as_str());
            market_telegram::ShareLink::Url(share)
        }
    };
    reply
        .buttons
        .push(vec![market_telegram::ReplyButton::Share {
            text: "Share".into(),
            destination,
        }]);
    reply
}
